/*
 * Yamaha ZG01 USB Audio Driver - PCM layer (single-card rework)
 *
 * One card, three PCM devices:
 *   pcm0 "ZG01 Game"      playback   - consumer of the EP 0x01 chain
 *   pcm1 "ZG01 Voice Out" playback   - consumer of the EP 0x01 chain
 *   pcm2 "ZG01 Voice In"  capture    - owner of the EP 0x81 chain
 *
 * The EP 0x01 URB chain is a single shared resource.  Every 240-byte
 * packet carries slots for BOTH playback consumers (voice L/R at byte
 * offsets 0-7, game L/R at 8-15, 24 pad bytes).  The chain runs while
 * either consumer needs it; the callback mixes each consumer's samples
 * from its own ALSA ring buffer (silence for a consumer that is not
 * running).  The ZG01 firmware performs the analog mix.
 */

#include <linux/module.h>
#include <linux/slab.h>
#include <linux/jiffies.h>
#include <sound/core.h>
#include <sound/pcm.h>
#include <sound/pcm_params.h>

#include "zg01.h"

#define PCM_BUFFER_BYTES_MAX_GAME   (1536 * 32)
#define PCM_BUFFER_BYTES_MIN_GAME   (1536 * 2)
#define PCM_PERIOD_BYTES_MIN_GAME   (192 * 8)
#define PCM_PERIOD_BYTES_MAX_GAME   (1536 * 8)

#define PCM_BUFFER_BYTES_MAX_VOICE  (48 * 32 * 64)
#define PCM_PERIOD_BYTES_MIN_VOICE  (48)
#define PCM_PERIOD_BYTES_MAX_VOICE  (48 * 16)

/* Delay before an idle chain kept alive by rapid START/STOP suppression
 * is quiesced for real (R7 fix). */
#define ZG01_QUIESCE_DELAY msecs_to_jiffies(200)

static struct zg01_stream *sub_to_stream(struct snd_pcm_substream *substream)
{
    return substream->pcm->private_data;
}

static const char *stream_name(const struct zg01_stream *s)
{
    return s->direction == SNDRV_PCM_STREAM_CAPTURE ? "Voice In" :
           (s->pcm_device == ZG01_PCM_GAME ? "Game" : "Voice Out");
}

/* Is any consumer of this chain currently running (state_mutex)? */
static bool chain_consumers_running(struct zg01_dev *dev, struct zg01_chain *c)
{
    if (c == &dev->out_chain)
        return dev->streams[ZG01_GAME].running ||
               dev->streams[ZG01_VOICE_OUT].running;
    return dev->streams[ZG01_VOICE_IN].running;
}

/* ================================================================== */
/* Deferred work handlers                                              */
/* ================================================================== */

void zg01_period_work_fn(struct work_struct *work)
{
    struct zg01_stream *s =
        container_of(work, struct zg01_stream, period_work);
    struct zg01_dev *dev = s->dev;
    struct snd_pcm_substream *sub;
    unsigned long flags;

    spin_lock_irqsave(&dev->lock, flags);
    sub = s->substream;
    spin_unlock_irqrestore(&dev->lock, flags);

    if (sub && !atomic_read(&dev->disconnecting))
        snd_pcm_period_elapsed(sub);
}

/* R6 fix: snd_pcm_stop_xrun() takes the PCM stream lock, which SLEEPS
 * for a nonatomic PCM.  Never call it from the URB callback. */
void zg01_xrun_work_fn(struct work_struct *work)
{
    struct zg01_stream *s = container_of(work, struct zg01_stream, xrun_work);
    struct zg01_dev *dev = s->dev;
    struct snd_pcm_substream *sub;
    unsigned long flags;

    spin_lock_irqsave(&dev->lock, flags);
    sub = s->substream;
    spin_unlock_irqrestore(&dev->lock, flags);

    if (!sub || atomic_read(&dev->disconnecting))
        return;

    snd_pcm_stream_lock_irqsave(sub, flags);
    if (snd_pcm_running(sub))
        snd_pcm_stop_xrun(sub);
    snd_pcm_stream_unlock_irqrestore(sub, flags);
}

void zg01_chain_cleanup_fn(struct work_struct *work)
{
    struct zg01_chain *c = container_of(work, struct zg01_chain, cleanup_work);
    struct zg01_dev *dev = c->dev;
    int i;

    /* usb_kill_urb() waits for every in-flight callback of this chain,
     * so after this loop no callback can touch dev or the streams. */
    for (i = 0; i < MAX_URBS; i++)
        if (c->urbs[i])
            usb_kill_urb(c->urbs[i]);

    mutex_lock(&dev->state_mutex);
    c->cleanup_pending = false;
    mutex_unlock(&dev->state_mutex);
}

/*
 * R7 fix: a rapid STOP/START burst that ends on a STOP leaves the chain
 * streaming silence forever.  Schedule a quiesce; a later START cancels
 * it asynchronously (the quiesce then no-ops on consumers-running).
 */
void zg01_chain_quiesce_fn(struct work_struct *work)
{
    struct zg01_chain *c = container_of(work, struct zg01_chain,
                                        quiesce_work.work);
    struct zg01_dev *dev = c->dev;
    int i;

    if (atomic_read(&dev->disconnecting))
        return;

    mutex_lock(&dev->state_mutex);
    if (c->allocated && atomic_read(&c->kill) == 0 &&
        atomic_read(&c->inflight) > 0 && !chain_consumers_running(dev, c)) {
        dev_dbg(&dev->udev->dev, "quiescing idle chain EP 0x%02x\n",
                c->endpoint);
        atomic_set(&c->kill, 1);
        for (i = 0; i < MAX_URBS; i++)
            if (c->urbs[i])
                usb_unlink_urb(c->urbs[i]);
        c->cleanup_pending = true;
        queue_work(zg01_cleanup_wq, &c->cleanup_work);
    }
    mutex_unlock(&dev->state_mutex);
}

/* ================================================================== */
/* Chain primitives (state_mutex side)                                 */
/* ================================================================== */

static void zg01_iso_out(struct urb *urb);
static void zg01_iso_in(struct urb *urb);

static int zg01_chain_alloc(struct zg01_dev *dev, struct zg01_chain *c,
                            unsigned int endpoint, unsigned int interface_num,
                            unsigned int iso_pkts, unsigned int iso_pkt_size,
                            bool playback)
{
    int i, k;

    if (c->allocated)
        return 0;

    c->dev = dev;
    c->endpoint = endpoint;
    c->interface_num = interface_num;
    c->iso_pkts = iso_pkts;
    c->iso_pkt_size = iso_pkt_size;

    for (i = 0; i < MAX_URBS; i++) {
        c->urbs[i] = usb_alloc_urb(iso_pkts, GFP_KERNEL);
        c->bufs[i] = kmalloc(iso_pkts * iso_pkt_size, GFP_KERNEL);
        if (!c->urbs[i] || !c->bufs[i]) {
            usb_free_urb(c->urbs[i]);
            kfree(c->bufs[i]);
            c->urbs[i] = NULL;
            c->bufs[i] = NULL;
            goto fail;
        }
        if (playback)
            memset(c->bufs[i], 0, iso_pkts * iso_pkt_size);
        c->urbs[i]->dev = dev->udev;
        c->urbs[i]->pipe = (endpoint & USB_DIR_IN)
            ? usb_rcvisocpipe(dev->udev, endpoint & 0x0F)
            : usb_sndisocpipe(dev->udev, endpoint & 0x0F);
        c->urbs[i]->transfer_buffer = c->bufs[i];
        c->urbs[i]->transfer_buffer_length = iso_pkts * iso_pkt_size;
        c->urbs[i]->complete = playback ? zg01_iso_out : zg01_iso_in;
        c->urbs[i]->context = c;
        c->urbs[i]->interval = 1;
        c->urbs[i]->start_frame = -1;
        c->urbs[i]->number_of_packets = iso_pkts;
        c->urbs[i]->transfer_flags = URB_ISO_ASAP;
        for (k = 0; k < iso_pkts; k++) {
            c->urbs[i]->iso_frame_desc[k].offset = k * iso_pkt_size;
            c->urbs[i]->iso_frame_desc[k].length = iso_pkt_size;
        }
    }

    c->allocated = true;
    return 0;

fail:
    for (k = 0; k < i; k++) {
        usb_free_urb(c->urbs[k]);
        kfree(c->bufs[k]);
        c->urbs[k] = NULL;
        c->bufs[k] = NULL;
    }
    return -ENOMEM;
}

/*
 * Stop a chain.  state_mutex may be held OR NOT (trigger paths hold it;
 * disconnect/suspend do not — the flag transitions are guarded either
 * way because every transition happens under state_mutex... callers
 * that do not hold it must take it around this call).
 *
 * Sets kill so callbacks stop resubmitting and drop the inflight count;
 * cleanup work performs the synchronous kills.
 */
static void zg01_chain_stop(struct zg01_dev *dev, struct zg01_chain *c)
{
    int i;

    lockdep_assert_held(&dev->state_mutex);

    if (!c->allocated)
        return;
    if (atomic_read(&c->kill))
        return;                    /* already stopped or stopping */

    atomic_set(&c->kill, 1);
    c->cleanup_pending = true;
    for (i = 0; i < MAX_URBS; i++)
        if (c->urbs[i])
            usb_unlink_urb(c->urbs[i]);
    queue_work(zg01_cleanup_wq, &c->cleanup_work);
}

/*
 * Start (or adopt) a chain.  Returns with the chain cycling.  Must be
 * called WITHOUT state_mutex held (it drops/reacquires around the
 * cleanup drain); callers re-check state after return.
 */
static int zg01_chain_start(struct zg01_dev *dev, struct zg01_chain *c)
{
    int submitted = 0;
    int i, ret;

    if (!c->allocated)
        return -ENOMEM;

    /* Async-cancel a pending quiesce; if it wins the race it no-ops
     * (consumer is running again).  Must NOT be _sync here: the
     * quiesce handler takes state_mutex. */
    cancel_delayed_work(&c->quiesce_work);

    mutex_lock(&dev->state_mutex);

    /* F2: never submit against a device being torn down — disconnect
     * frees the URBs under this mutex after setting disconnecting. */
    if (atomic_read(&dev->disconnecting)) {
        mutex_unlock(&dev->state_mutex);
        return -ENODEV;
    }

    if (c->cleanup_pending) {
        mutex_unlock(&dev->state_mutex);
        flush_work(&c->cleanup_work);   /* also drains callbacks */
        mutex_lock(&dev->state_mutex);
        c->cleanup_pending = false;
        if (atomic_read(&dev->disconnecting) || !c->allocated) {
            mutex_unlock(&dev->state_mutex);
            return -ENODEV;
        }
    }

    if (atomic_read(&c->inflight) > 0) {
        mutex_unlock(&dev->state_mutex);
        return 0;                        /* adopt live chain */
    }

    atomic_set(&c->kill, 0);
    atomic_set(&c->inflight, MAX_URBS);

    /* Submit with state_mutex held: disconnect's URB free path also
     * runs under state_mutex, so a URB can never be freed between
     * our check and usb_submit_urb. */
    for (i = 0; i < MAX_URBS; i++) {
        ret = usb_submit_urb(c->urbs[i], GFP_KERNEL);
        if (ret)
            goto fail;
        submitted++;
    }

    mutex_unlock(&dev->state_mutex);
    return 0;

fail:
    /* F1: URBs never handed to usb_submit_urb can never decrement
     * inflight via a completion — subtract them here, or the counter
     * sticks >0 and a later start "adopts" a dead chain forever. */
    atomic_sub(MAX_URBS - submitted, &c->inflight);
    atomic_set(&c->kill, 1);
    c->cleanup_pending = true;
    for (i = submitted - 1; i >= 0; i--)
        usb_unlink_urb(c->urbs[i]);
    queue_work(zg01_cleanup_wq, &c->cleanup_work);
    mutex_unlock(&dev->state_mutex);
    return ret;
}

/* Free URBs and buffers for real (hw_free / disconnect, after drain). */
static void __maybe_unused zg01_chain_free(struct zg01_chain *c)
{
    int i;

    for (i = 0; i < MAX_URBS; i++) {
        if (c->urbs[i]) {
            usb_kill_urb(c->urbs[i]);
            usb_free_urb(c->urbs[i]);
            c->urbs[i] = NULL;
        }
        kfree(c->bufs[i]);
        c->bufs[i] = NULL;
    }
    c->allocated = false;
}

void zg01_stop_all_chains(struct zg01_dev *dev)
{
    /* cancel_sync must run OUTSIDE state_mutex: the quiesce handler
     * takes it (deadlock otherwise). */
    cancel_delayed_work_sync(&dev->out_chain.quiesce_work);
    cancel_delayed_work_sync(&dev->in_chain.quiesce_work);
    mutex_lock(&dev->state_mutex);
    zg01_chain_stop(dev, &dev->out_chain);
    zg01_chain_stop(dev, &dev->in_chain);
    mutex_unlock(&dev->state_mutex);
}

void zg01_drain_all_chains(struct zg01_dev *dev)
{
    flush_work(&dev->out_chain.cleanup_work);
    flush_work(&dev->in_chain.cleanup_work);
}

/* ================================================================== */
/* Rate / vendor "Magic Sequence"                                      */
/* ================================================================== */

/*
 * Resets BOTH interfaces to alt 0 — kills every live URB.  Callers must
 * hold state_mutex and have verified no chain is streaming.
 */
int zg01_set_rate(struct zg01_dev *dev, int rate)
{
    unsigned char *data;
    unsigned char *large_data;
    int ret = 0;

    if (!dev || !dev->udev)
        return -ENODEV;

    data = kmalloc(4, GFP_KERNEL);
    large_data = kmalloc(72, GFP_KERNEL);
    if (!data || !large_data) {
        kfree(data);
        kfree(large_data);
        return -ENOMEM;
    }

    dev_dbg(&dev->udev->dev, "set_rate %d\n", rate);

    /* 1. Early vendor reads (state discovery / standby exit) */
    usb_control_msg(dev->udev, usb_rcvctrlpipe(dev->udev, 0),
                    0x07, USB_DIR_IN | USB_TYPE_VENDOR | USB_RECIP_DEVICE,
                    0x0000, 0x0000, large_data, 3, 1000);
    usb_control_msg(dev->udev, usb_rcvctrlpipe(dev->udev, 0),
                    0x04, USB_DIR_IN | USB_TYPE_VENDOR | USB_RECIP_DEVICE,
                    0x0000, 0x0000, large_data, 1, 1000);
    usb_control_msg(dev->udev, usb_rcvctrlpipe(dev->udev, 0),
                    0x0a, USB_DIR_IN | USB_TYPE_VENDOR | USB_RECIP_DEVICE,
                    0x0000, 0x0000, large_data, 4, 1000);
    usb_control_msg(dev->udev, usb_rcvctrlpipe(dev->udev, 0),
                    0x0c, USB_DIR_IN | USB_TYPE_VENDOR | USB_RECIP_DEVICE,
                    0x8000, 0x0000, large_data, 72, 1000);
    usb_control_msg(dev->udev, usb_rcvctrlpipe(dev->udev, 0),
                    0x0c, USB_DIR_IN | USB_TYPE_VENDOR | USB_RECIP_DEVICE,
                    0x0000, 0x0000, large_data, 72, 1000);

    /* 2. Interfaces to alt 0 */
    usb_set_interface(dev->udev, 1, 0);
    usb_set_interface(dev->udev, 2, 0);

    /* 3. UAC2 SET_CUR on clock source 1, verify with GET_CUR */
    data[0] = rate & 0xff;
    data[1] = (rate >> 8) & 0xff;
    data[2] = (rate >> 16) & 0xff;
    data[3] = (rate >> 24) & 0xff;
    {
        int attempt;

        for (attempt = 1; attempt <= 3; attempt++) {
            ret = usb_control_msg(dev->udev, usb_sndctrlpipe(dev->udev, 0),
                                  0x01, USB_DIR_OUT | USB_TYPE_CLASS | USB_RECIP_INTERFACE,
                                  0x0100, 0x0100, data, 4, 1000);
            if (ret < 0)
                dev_warn(&dev->udev->dev, "set_rate attempt %d SET_CUR: %d\n",
                         attempt, ret);

            ret = usb_control_msg(dev->udev, usb_rcvctrlpipe(dev->udev, 0),
                                  0x01, USB_DIR_IN | USB_TYPE_CLASS | USB_RECIP_INTERFACE,
                                  0x0100, 0x0100, large_data, 4, 1000);
            if (ret == 4) {
                unsigned int ret_rate = (u32)large_data[0] |
                                        ((u32)large_data[1] << 8) |
                                        ((u32)large_data[2] << 16) |
                                        ((u32)large_data[3] << 24);

                dev->current_rate = ret_rate;
                ret = 0;
                if ((int)ret_rate != rate)
                    dev_warn(&dev->udev->dev,
                             "device reported rate %u (asked %d), using device rate\n",
                             ret_rate, rate);
                break;
            }
            ret = (ret < 0) ? ret : -EIO;
            if (attempt < 3)
                msleep(150);
        }
    }

    /* 4. Commit handshake */
    usb_control_msg(dev->udev, usb_rcvctrlpipe(dev->udev, 0),
                    0x02, USB_DIR_IN | USB_TYPE_VENDOR | USB_RECIP_DEVICE,
                    0x0002, 0x0000, large_data, 1, 1000);
    usb_control_msg(dev->udev, usb_rcvctrlpipe(dev->udev, 0),
                    0x02, USB_DIR_IN | USB_TYPE_VENDOR | USB_RECIP_DEVICE,
                    0x0001, 0x0000, large_data, 1, 1000);
    usb_control_msg(dev->udev, usb_rcvctrlpipe(dev->udev, 0),
                    0x08, USB_DIR_IN | USB_TYPE_VENDOR | USB_RECIP_DEVICE,
                    0x0000, 0x0000, large_data, 1, 1000);
    usb_control_msg(dev->udev, usb_sndctrlpipe(dev->udev, 0),
                    0x00, USB_DIR_OUT | USB_TYPE_VENDOR | USB_RECIP_INTERFACE,
                    0x0000, 0x0000, NULL, 0, 1000);

    /* 5. Streaming interfaces back to alt 1 */
    usb_set_interface(dev->udev, 1, 1);
    usb_set_interface(dev->udev, 2, 1);

    msleep(200);

    kfree(large_data);
    kfree(data);
    return ret;
}

/* ================================================================== */
/* ISO callbacks                                                       */
/* ================================================================== */

/* Resubmit gate.  Returns true when the URB was resubmitted; on false
 * the caller must NOT touch the chain again (inflight already dropped). */
static bool chain_resubmit(struct zg01_chain *c, struct urb *urb)
{
    struct zg01_dev *dev = c->dev;
    int i, ret;

    if (atomic_read(&dev->disconnecting) || atomic_read(&c->kill) ||
        urb->status == -ESHUTDOWN || urb->status == -ENOENT ||
        urb->status == -ECONNRESET) {
        atomic_dec(&c->inflight);
        return false;
    }

    for (i = 0; i < urb->number_of_packets; i++) {
        urb->iso_frame_desc[i].status = 0;
        urb->iso_frame_desc[i].actual_length = 0;
    }

    ret = usb_submit_urb(urb, GFP_ATOMIC);
    if (ret < 0) {
        dev_warn(&dev->udev->dev, "resubmit failed on EP 0x%02x: %d\n",
                 c->endpoint, ret);
        atomic_dec(&c->inflight);
        /* F3: surface the dead chain to ALSA as an xrun instead of
         * clicking silently forever.  Queuing work from softirq is
         * safe; the handlers check substream state themselves. */
        if (c == &dev->out_chain) {
            if (dev->streams[ZG01_GAME].running)
                queue_work(zg01_period_wq,
                           &dev->streams[ZG01_GAME].xrun_work);
            if (dev->streams[ZG01_VOICE_OUT].running)
                queue_work(zg01_period_wq,
                           &dev->streams[ZG01_VOICE_OUT].xrun_work);
        } else if (dev->streams[ZG01_VOICE_IN].running) {
            queue_work(zg01_period_wq, &dev->streams[ZG01_VOICE_IN].xrun_work);
        }
        return false;
    }
    return true;
}

/*
 * Clamp a callback-side position advance to the appl_ptr budget.
 * budget = appl_ptr - appl_base; pcm_pos may never pass the last frame
 * userspace committed.  s64 compare only — a u32 subtraction wraps and
 * silently disarms the clamp after a rewind.
 */
static unsigned int clamp_advance(struct zg01_stream *s,
                                  struct snd_pcm_runtime *rt,
                                  unsigned int advance)
{
    if (rt->control) {
        u64 appl = READ_ONCE(rt->control->appl_ptr);
        u64 base = READ_ONCE(s->appl_base);
        s64 budget = (s64)(appl - base);
        s64 headroom;

        if (budget < 0) {
            /* appl_ptr moved backwards (RESET sets appl = hw): rebase
             * with zero headroom; the next userspace write resumes. */
            WRITE_ONCE(s->appl_base, appl - s->pcm_pos);
            budget = (s64)s->pcm_pos;
        }
        headroom = budget - (s64)s->pcm_pos;
        if (headroom < (s64)advance)
            return headroom > 0 ? (unsigned int)headroom : 0;
    }
    return advance;
}

/* Snapshot of one playback consumer, taken under dev->lock. */
struct out_consumer {
    struct zg01_stream *s;
    struct snd_pcm_runtime *rt;
    bool active;              /* has ring + stream running */
    unsigned int old_pos;
    bool period_elapsed;
};

static void snap_consumer(struct zg01_dev *dev, enum zg01_stream_id id,
                          struct out_consumer *oc)
{
    struct zg01_stream *s = &dev->streams[id];
    struct snd_pcm_substream *sub = s->substream;

    oc->s = s;
    oc->rt = NULL;
    oc->active = false;
    oc->period_elapsed = false;
    oc->old_pos = s->pcm_pos;

    if (sub && sub->runtime && sub->runtime->dma_area &&
        snd_pcm_running(sub)) {
        oc->rt = sub->runtime;
        oc->active = true;
    }
}

/* Read one interleaved stereo S32 frame from a ring buffer. */
static void read_frame(struct snd_pcm_runtime *rt, unsigned int frame,
                       u32 *l, u32 *r)
{
    unsigned int bpf = rt->frame_bits / 8;
    unsigned int buf_bytes = rt->buffer_size * bpf;
    unsigned int off = (frame % rt->buffer_size) * bpf;
    u8 tmp[8];

    if (off + 8 <= buf_bytes) {
        memcpy(l, rt->dma_area + off, 4);
        memcpy(r, rt->dma_area + off + 4, 4);
    } else {
        unsigned int first = buf_bytes - off;
        memcpy(tmp, rt->dma_area + off, first);
        memcpy(tmp + first, rt->dma_area, 8 - first);
        memcpy(l, tmp, 4);
        memcpy(r, tmp + 4, 4);
    }
}

/* Advance a consumer under dev->lock after total_frames consumed. */
static void advance_consumer(struct out_consumer *oc, unsigned int total)
{
    struct zg01_stream *s = oc->s;
    unsigned int period_size = oc->rt->period_size;
    unsigned int advance;

    if (!oc->active || total == 0)
        return;

    advance = clamp_advance(s, oc->rt, total);
    if (advance == 0)
        return;

    s->pcm_pos += advance;
    if (period_size > 0 &&
        (s->pcm_pos / period_size) != (oc->old_pos / period_size))
        oc->period_elapsed = true;
}

/*
 * EP 0x01 OUT completion: mix both playback consumers into the packet
 * slots and advance their positions (clamped).  All ring accesses and
 * position updates happen under dev->lock; every clear of those
 * pointers in close/hw_free also happens under dev->lock AFTER the
 * chain drain — no window where the callback touches freed memory
 * (R2 closed by construction: one dev, one lock, drain before clear).
 */
static void zg01_iso_out(struct urb *urb)
{
    struct zg01_chain *c = urb->context;
    struct zg01_dev *dev = c->dev;
    struct out_consumer game = {0}, vo = {0};
    unsigned long flags;
    unsigned int total_frames = 0;
    int i, f;

    if (urb->status && urb->status != -EXDEV)
        dev_warn_ratelimited(&dev->udev->dev, "out URB status %d\n",
                             urb->status);

    spin_lock_irqsave(&dev->lock, flags);

    if (urb->status == 0) {
        snap_consumer(dev, ZG01_GAME, &game);
        snap_consumer(dev, ZG01_VOICE_OUT, &vo);

        for (i = 0; i < urb->number_of_packets; i++) {
            unsigned int pkt_len = urb->iso_frame_desc[i].length;
            u8 *pkt;

            if (pkt_len != 240)
                continue;

            pkt = urb->transfer_buffer + urb->iso_frame_desc[i].offset;

            for (f = 0; f < 6; f++) {
                u32 gl = 0, gr = 0, vl = 0, vr = 0;
                u8 *slot = pkt + f * 40;

                if (game.active)
                    read_frame(game.rt, game.s->pcm_pos + total_frames + f,
                               &gl, &gr);
                if (vo.active)
                    read_frame(vo.rt, vo.s->pcm_pos + total_frames + f,
                               &vl, &vr);

                memcpy(slot, &vl, 4);
                memcpy(slot + 4, &vr, 4);
                memcpy(slot + 8, &gl, 4);
                memcpy(slot + 12, &gr, 4);
                memset(slot + 16, 0, 24);
            }
            total_frames += 6;
        }

        advance_consumer(&game, total_frames);
        advance_consumer(&vo, total_frames);
    }

    spin_unlock_irqrestore(&dev->lock, flags);

    if (game.period_elapsed)
        queue_work(zg01_period_wq, &game.s->period_work);
    if (vo.period_elapsed)
        queue_work(zg01_period_wq, &vo.s->period_work);

    chain_resubmit(c, urb);
}

/*
 * EP 0x81 IN completion: Voice In capture.  Packet format (108 bytes):
 *   8-byte header, 6 frames x 16 bytes (L4 R4 + 8 pad), 4-byte trailer.
 */
static void zg01_iso_in(struct urb *urb)
{
    struct zg01_chain *c = urb->context;
    struct zg01_dev *dev = c->dev;
    struct zg01_stream *s = &dev->streams[ZG01_VOICE_IN];
    struct snd_pcm_substream *sub;
    struct snd_pcm_runtime *rt;
    unsigned long flags;
    unsigned int old_pos = 0;
    bool period_elapsed = false;
    int i, f;

    if (urb->status && urb->status != -EXDEV)
        dev_warn_ratelimited(&dev->udev->dev, "in URB status %d\n",
                             urb->status);

    spin_lock_irqsave(&dev->lock, flags);

    sub = s->substream;
    rt = (sub && sub->runtime && sub->runtime->dma_area &&
          snd_pcm_running(sub)) ? sub->runtime : NULL;

    if (urb->status == 0 && rt) {
        unsigned int bpf = rt->frame_bits / 8;
        unsigned int buf_bytes = rt->buffer_size * bpf;
        unsigned int period_size = rt->period_size;

        old_pos = s->pcm_pos;

        for (i = 0; i < urb->number_of_packets; i++) {
            unsigned int pkt_len = urb->iso_frame_desc[i].actual_length;
            u8 *pkt;
            unsigned int write_frame;
            unsigned int write_off;

            if (pkt_len != 108)
                continue;

            pkt = urb->transfer_buffer + urb->iso_frame_desc[i].offset;

            for (f = 0; f < 6; f++) {
                u8 *usb_frame = pkt + 8 + f * 16;

                write_frame = s->pcm_pos % rt->buffer_size;
                write_off = write_frame * bpf;
                if (write_off + 8 <= buf_bytes) {
                    memcpy(rt->dma_area + write_off, usb_frame, 8);
                } else {
                    unsigned int first = buf_bytes - write_off;
                    memcpy(rt->dma_area + write_off, usb_frame, first);
                    memcpy(rt->dma_area, usb_frame + first, 8 - first);
                }
                s->pcm_pos++;
            }
        }

        if (period_size > 0 &&
            (s->pcm_pos / period_size) != (old_pos / period_size))
            period_elapsed = true;
    }

    spin_unlock_irqrestore(&dev->lock, flags);

    if (period_elapsed)
        queue_work(zg01_period_wq, &s->period_work);

    chain_resubmit(c, urb);
}

/* ================================================================== */
/* PCM operations                                                      */
/* ================================================================== */

static int zg01_pcm_open(struct snd_pcm_substream *substream)
{
    struct zg01_stream *s = sub_to_stream(substream);
    struct zg01_dev *dev = s->dev;
    struct snd_pcm_runtime *runtime = substream->runtime;
    unsigned long now = jiffies;
    bool rapid;
    int ret = 0;

    mutex_lock(&dev->state_mutex);

    if (atomic_read(&dev->disconnecting)) {
        ret = -ENODEV;
        goto unlock;
    }

    /* Log rate limiting for audio-system probing */
    if (time_before(now, dev->last_open_jiffies + msecs_to_jiffies(1000)))
        dev->open_count++;
    else
        dev->open_count = 1;
    dev->last_open_jiffies = now;
    rapid = dev->open_count > 2;

    runtime->hw.info = SNDRV_PCM_INFO_MMAP | SNDRV_PCM_INFO_MMAP_VALID |
                       SNDRV_PCM_INFO_INTERLEAVED |
                       SNDRV_PCM_INFO_BLOCK_TRANSFER | SNDRV_PCM_INFO_BATCH;
    runtime->hw.formats = SNDRV_PCM_FMTBIT_S32_LE;
    runtime->hw.channels_min = 2;
    runtime->hw.channels_max = 2;
    runtime->hw.periods_min = 2;
    runtime->hw.periods_max = 64;

    if (s->direction == SNDRV_PCM_STREAM_CAPTURE) {
        runtime->hw.rates = SNDRV_PCM_RATE_48000 | SNDRV_PCM_RATE_16000;
        runtime->hw.rate_min = 16000;
        runtime->hw.rate_max = 48000;
        runtime->hw.buffer_bytes_max = PCM_BUFFER_BYTES_MAX_VOICE;
        runtime->hw.period_bytes_min = PCM_PERIOD_BYTES_MIN_VOICE;
        runtime->hw.period_bytes_max = PCM_PERIOD_BYTES_MAX_VOICE;
        ret = snd_pcm_hw_constraint_step(runtime, 0,
                                         SNDRV_PCM_HW_PARAM_PERIOD_BYTES, 48);
        if (ret)
            goto unlock;
        ret = snd_pcm_hw_constraint_step(runtime, 0,
                                         SNDRV_PCM_HW_PARAM_BUFFER_BYTES, 48);
    } else {
        runtime->hw.rates = SNDRV_PCM_RATE_48000;
        runtime->hw.rate_min = 48000;
        runtime->hw.rate_max = 48000;
        runtime->hw.buffer_bytes_max = PCM_BUFFER_BYTES_MAX_GAME;
        runtime->hw.period_bytes_min = PCM_PERIOD_BYTES_MIN_GAME;
        runtime->hw.period_bytes_max = PCM_PERIOD_BYTES_MAX_GAME;
        ret = snd_pcm_hw_constraint_step(runtime, 0,
                                         SNDRV_PCM_HW_PARAM_PERIOD_BYTES, 1536);
        if (ret)
            goto unlock;
        ret = snd_pcm_hw_constraint_step(runtime, 0,
                                         SNDRV_PCM_HW_PARAM_BUFFER_BYTES, 96);
    }
    if (ret)
        goto unlock;

    s->opened = true;
    dev_info(&dev->udev->dev, "open %s\n", stream_name(s));

unlock:
    mutex_unlock(&dev->state_mutex);
    return ret;
}

/*
 * close: stop this stream's claim on its chain, drain everything that
 * can reference the substream, then clear the pointer.  The drain runs
 * WITHOUT state_mutex (cleanup work re-acquires it), the pointer clear
 * runs under dev->lock — after the drain no callback or queued work
 * can still hold the old substream (R2/MR1 closed).
 */
static int zg01_pcm_close(struct snd_pcm_substream *substream)
{
    struct zg01_stream *s = sub_to_stream(substream);
    struct zg01_dev *dev = s->dev;
    struct zg01_chain *c = (s->direction == SNDRV_PCM_STREAM_CAPTURE)
        ? &dev->in_chain : &dev->out_chain;
    unsigned long flags;

    mutex_lock(&dev->state_mutex);
    s->opened = false;
    s->running = false;
    if (!chain_consumers_running(dev, c))
        zg01_chain_stop(dev, c);
    mutex_unlock(&dev->state_mutex);

    flush_work(&c->cleanup_work);
    cancel_delayed_work_sync(&c->quiesce_work);
    flush_work(&s->period_work);
    flush_work(&s->xrun_work);

    spin_lock_irqsave(&dev->lock, flags);
    s->substream = NULL;
    spin_unlock_irqrestore(&dev->lock, flags);

    return 0;
}

static int zg01_pcm_hw_params(struct snd_pcm_substream *substream,
                              struct snd_pcm_hw_params *hw_params)
{
    if (params_channels(hw_params) != 2)
        return -EINVAL;
    if (params_format(hw_params) != SNDRV_PCM_FORMAT_S32_LE)
        return -EINVAL;
    if (params_rate(hw_params) != 48000 && params_rate(hw_params) != 16000)
        return -EINVAL;

    return 0;
}

static int zg01_pcm_hw_free(struct snd_pcm_substream *substream)
{
    struct zg01_stream *s = sub_to_stream(substream);
    struct zg01_dev *dev = s->dev;
    struct zg01_chain *c = (s->direction == SNDRV_PCM_STREAM_CAPTURE)
        ? &dev->in_chain : &dev->out_chain;
    unsigned long flags;

    mutex_lock(&dev->state_mutex);
    s->running = false;
    if (!chain_consumers_running(dev, c))
        zg01_chain_stop(dev, c);
    mutex_unlock(&dev->state_mutex);

    flush_work(&c->cleanup_work);
    flush_work(&s->period_work);
    flush_work(&s->xrun_work);

    /* dma_area is freed after this callback returns; make sure no
     * callback can still read it. */
    spin_lock_irqsave(&dev->lock, flags);
    s->substream = NULL;
    spin_unlock_irqrestore(&dev->lock, flags);

    /* Keep the chains allocated: next prepare reuses them. */
    return 0;
}

static int zg01_pcm_prepare(struct snd_pcm_substream *substream)
{
    struct zg01_stream *s = sub_to_stream(substream);
    struct zg01_dev *dev = s->dev;
    unsigned long flags;
    int ret;

    mutex_lock(&dev->state_mutex);

    if (atomic_read(&dev->disconnecting)) {
        mutex_unlock(&dev->state_mutex);
        return -ENODEV;
    }

    /*
     * Device initialization (vendor handshake + rate).  The Magic
     * Sequence resets both interfaces to alt 0, killing every live URB
     * — run it only when NO chain is streaming.  state_mutex makes this
     * check atomic against triggers on the sibling PCMs (R1).
     *
     * F4: the clock is DEVICE-WIDE (one UAC2 clock source shared by
     * both interfaces).  Always initialize at 48 kHz — a Voice In
     * first-open at 16 kHz must not mis-clock the 48 kHz-only Game
     * playback (old driver effectively behaved the same: first init
     * always requested 48000).
     */
    if (!dev->device_initialized &&
        atomic_read(&dev->out_chain.inflight) == 0 &&
        atomic_read(&dev->in_chain.inflight) == 0) {
        ret = zg01_set_rate(dev, 48000);
        if (ret < 0)
            dev_warn(&dev->udev->dev, "set_rate failed: %d\n", ret);
        dev->device_initialized = true;
    }

    /*
     * Ensure this stream's streaming interface is at alt 1.  Safe now:
     * state_mutex excludes concurrent triggers on the sibling PCMs, so
     * a live chain cannot be flushed by this call (usb_set_interface
     * unlinks all URBs on the target interface, even same-alt).
     */
    if (s->direction == SNDRV_PCM_STREAM_CAPTURE) {
        if (atomic_read(&dev->in_chain.inflight) == 0)
            usb_set_interface(dev->udev, 2, 1);
    } else {
        if (atomic_read(&dev->out_chain.inflight) == 0)
            usb_set_interface(dev->udev, 1, 1);
    }

    /* URB pre-allocation for this stream's chain */
    if (s->direction == SNDRV_PCM_STREAM_CAPTURE)
        ret = zg01_chain_alloc(dev, &dev->in_chain, ZG01_EP_IN, 2,
                               ISO_PKTS_IN, ISO_PKT_SIZE_IN, false);
    else
        ret = zg01_chain_alloc(dev, &dev->out_chain, ZG01_EP_OUT, 1,
                               ISO_PKTS_OUT, ISO_PKT_SIZE_OUT, true);
    if (ret) {
        mutex_unlock(&dev->state_mutex);
        return ret;
    }

    /*
     * Publish the substream and reset the position under dev->lock.
     * For playback capture the appl_ptr epoch AFTER the reset:
     *   base = hw_ptr - 0  =>  budget = appl_ptr - hw_ptr = queued.
     */
    spin_lock_irqsave(&dev->lock, flags);
    s->substream = substream;
    s->pcm_pos = 0;
    if (s->direction == SNDRV_PCM_STREAM_PLAYBACK &&
        substream->runtime && substream->runtime->status)
        s->appl_base = substream->runtime->status->hw_ptr;
    spin_unlock_irqrestore(&dev->lock, flags);

    s->initialized = true;

    mutex_unlock(&dev->state_mutex);
    return 0;
}

static int zg01_pcm_trigger(struct snd_pcm_substream *substream, int cmd)
{
    struct zg01_stream *s = sub_to_stream(substream);
    struct zg01_dev *dev = s->dev;
    struct zg01_chain *c = (s->direction == SNDRV_PCM_STREAM_CAPTURE)
        ? &dev->in_chain : &dev->out_chain;
    bool rapid;
    int ret;

    mutex_lock(&dev->state_mutex);

    switch (cmd) {
    case SNDRV_PCM_TRIGGER_START:
        if (!c->allocated) {
            mutex_unlock(&dev->state_mutex);
            return -ENOMEM;
        }
        s->running = true;
        mutex_unlock(&dev->state_mutex);

        /* May sleep (cleanup drain); state re-checked inside. */
        ret = zg01_chain_start(dev, c);
        if (ret) {
            mutex_lock(&dev->state_mutex);
            s->running = false;
            mutex_unlock(&dev->state_mutex);
            dev_err(&dev->udev->dev, "chain start failed: %d\n", ret);
        }
        return ret;

    case SNDRV_PCM_TRIGGER_STOP:
        /* Rapid START/STOP burst detection (PipeWire reconfiguration) */
        if (time_before(jiffies,
                        s->last_trigger_jiffies + msecs_to_jiffies(100)))
            s->trigger_count++;
        else
            s->trigger_count = 1;
        s->last_trigger_jiffies = jiffies;
        rapid = s->trigger_count > 3;

        s->running = false;
        if (rapid) {
            /* Keep the chain cycling (silence); the quiesce timer
             * stops it if no START follows (R7). */
            mod_delayed_work(zg01_cleanup_wq, &c->quiesce_work,
                             ZG01_QUIESCE_DELAY);
            mutex_unlock(&dev->state_mutex);
            return 0;
        }
        if (!chain_consumers_running(dev, c))
            zg01_chain_stop(dev, c);
        mutex_unlock(&dev->state_mutex);
        return 0;

    case SNDRV_PCM_TRIGGER_SUSPEND:
        s->running = false;
        if (!chain_consumers_running(dev, c))
            zg01_chain_stop(dev, c);
        mutex_unlock(&dev->state_mutex);
        return 0;

    default:
        mutex_unlock(&dev->state_mutex);
        return -EINVAL;
    }
}

static snd_pcm_uframes_t zg01_pcm_pointer(struct snd_pcm_substream *substream)
{
    struct zg01_stream *s = sub_to_stream(substream);
    struct zg01_dev *dev = s->dev;
    unsigned long pos;
    unsigned long flags;

    spin_lock_irqsave(&dev->lock, flags);
    pos = s->pcm_pos;
    spin_unlock_irqrestore(&dev->lock, flags);

    return pos % substream->runtime->buffer_size;
}

static int zg01_pcm_ioctl(struct snd_pcm_substream *substream,
                          unsigned int cmd, void *arg)
{
    return snd_pcm_lib_ioctl(substream, cmd, arg);
}

static const struct snd_pcm_ops zg01_pcm_ops = {
    .open = zg01_pcm_open,
    .close = zg01_pcm_close,
    .ioctl = zg01_pcm_ioctl,
    .hw_params = zg01_pcm_hw_params,
    .hw_free = zg01_pcm_hw_free,
    .prepare = zg01_pcm_prepare,
    .trigger = zg01_pcm_trigger,
    .pointer = zg01_pcm_pointer,
};

/* ================================================================== */
/* Card / PCM creation                                                 */
/* ================================================================== */

static int zg01_new_pcm(struct zg01_dev *dev, enum zg01_stream_id id,
                        const char *name, int playback, int capture,
                        int buffer_bytes_max)
{
    struct zg01_stream *s = &dev->streams[id];
    struct snd_pcm *pcm;
    int ret;

    ret = snd_pcm_new(dev->card, name, s->pcm_device, playback, capture, &pcm);
    if (ret)
        return ret;

    snd_pcm_set_ops(pcm, playback ? SNDRV_PCM_STREAM_PLAYBACK
                                  : SNDRV_PCM_STREAM_CAPTURE, &zg01_pcm_ops);
    pcm->private_data = s;
    pcm->nonatomic = 1;
    strscpy(pcm->name, name, sizeof(pcm->name));

    dev->pcm_instances[id] = pcm;
    return 0;
}

int zg01_create_pcm_devices(struct zg01_dev *dev)
{
    int ret;

    ret = zg01_new_pcm(dev, ZG01_GAME, "ZG01 Game", 1, 0,
                       PCM_BUFFER_BYTES_MAX_GAME);
    if (ret)
        return ret;

    ret = zg01_new_pcm(dev, ZG01_VOICE_OUT, "ZG01 Voice Out", 1, 0,
                       PCM_BUFFER_BYTES_MAX_GAME);
    if (ret)
        return ret;

    ret = zg01_new_pcm(dev, ZG01_VOICE_IN, "ZG01 Voice In", 0, 1,
                       PCM_BUFFER_BYTES_MAX_VOICE);
    if (ret)
        return ret;

    for (int i = 0; i < ZG01_N_STREAMS; i++) {
        unsigned int max = dev->streams[i].direction ==
                           SNDRV_PCM_STREAM_CAPTURE
                               ? PCM_BUFFER_BYTES_MAX_VOICE
                               : PCM_BUFFER_BYTES_MAX_GAME;

        /* Match old driver: managed minimum = max/8 (6144 game/VO,
         * 12288 voice-in), not the smaller hw floor. */
        snd_pcm_set_managed_buffer_all(dev->pcm_instances[i],
                                       SNDRV_DMA_TYPE_CONTINUOUS, NULL,
                                       max / 8, max);
    }

    return 0;
}

/* ================================================================== */
/* PM helpers (called from zg01_usb.c)                                 */
/* ================================================================== */

void zg01_suspend_pcm(struct zg01_dev *dev)
{
    int i;

    for (i = 0; i < ZG01_N_STREAMS; i++) {
        if (dev->pcm_instances[i])
            snd_pcm_suspend_all(dev->pcm_instances[i]);
    }
}

void zg01_pm_reset_streams(struct zg01_dev *dev)
{
    int i;

    /*
     * The firmware reset itself across suspend: force the first-prepare
     * path (vendor handshake + rate) on the next open of each stream.
     * Piggyback/keepalive state no longer exists — chain state is
     * fully described by the kill/inflight atomics, which the suspend
     * stop already quiesced (R5 closed by construction).
     */
    mutex_lock(&dev->state_mutex);
    for (i = 0; i < ZG01_N_STREAMS; i++) {
        dev->streams[i].running = false;
        dev->streams[i].initialized = false;
    }
    dev->device_initialized = false;
    mutex_unlock(&dev->state_mutex);
}

MODULE_LICENSE("GPL");
MODULE_DESCRIPTION("Yamaha ZG01 USB Audio Driver - PCM layer");
