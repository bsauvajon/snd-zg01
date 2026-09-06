/*
 * Yamaha ZG01 USB Audio Driver - PCM layer
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
#include <linux/ktime.h>
#include <sound/info.h>
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
 * is quiesced for real. */
#define ZG01_QUIESCE_DELAY msecs_to_jiffies(200)

static struct zg01_stream *sub_to_stream(struct snd_pcm_substream *substream)
{
    return substream->pcm->private_data;
}

static const char *stream_name(const struct zg01_stream *s)
{
    return s->direction == SNDRV_PCM_STREAM_CAPTURE ? "Voice In" :
           (s->pcm_device == ZG01_PCM_GAME ? "Game Out" : "Voice Out");
}

/* Is any consumer of this chain currently running (state_mutex)? */
static bool chain_consumers_running(struct zg01_dev *dev, struct zg01_chain *c)
{
    if (c == &dev->out_chain)
        return dev->streams[ZG01_GAME].running ||
               dev->streams[ZG01_VOICE_OUT].running;
    return dev->streams[ZG01_VOICE_IN].running ||
           dev->streams[ZG01_GAME].running ||
           dev->streams[ZG01_VOICE_OUT].running ||
           (dev->out_chain.allocated && !atomic_read(&dev->out_chain.kill));
}

static void zg01_chain_stop(struct zg01_dev *dev, struct zg01_chain *c);
static void zg01_feedback_pump(struct zg01_dev *dev);
static void zg01_feedback_xrun(struct zg01_dev *dev);

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

/* snd_pcm_stop_xrun() takes the PCM stream lock, which SLEEPS
 * for a nonatomic PCM.  Never call it from the URB callback. */
void zg01_xrun_work_fn(struct work_struct *work)
{
    struct zg01_stream *s = container_of(work, struct zg01_stream, xrun_work);
    struct zg01_dev *dev = s->dev;
    struct snd_pcm_substream *sub;
    unsigned long flags, pcm_flags;
    unsigned int generation;
    bool stop;

    spin_lock_irqsave(&dev->lock, flags);
    sub = s->substream;
    generation = s->xrun_generation;
    spin_unlock_irqrestore(&dev->lock, flags);
    if (!sub || !generation || atomic_read(&dev->disconnecting))
        return;

    /* Serialize the generation check with ALSA prepare/trigger. Never hold
     * dev->lock while taking the (nonatomic, sleeping) PCM stream lock. */
    snd_pcm_stream_lock_irqsave(sub, pcm_flags);
    spin_lock_irqsave(&dev->lock, flags);
    stop = s->substream == sub && s->enabled && s->generation == generation;
    spin_unlock_irqrestore(&dev->lock, flags);
    if (stop && snd_pcm_running(sub))
        snd_pcm_stop(sub, SNDRV_PCM_STATE_XRUN);
    snd_pcm_stream_unlock_irqrestore(sub, pcm_flags);
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
 * A rapid STOP/START burst that ends on a STOP leaves the chain
 * streaming silence forever.  Schedule a quiesce; a later START cancels
 * it asynchronously (the quiesce then no-ops on consumers-running).
 */
void zg01_chain_quiesce_fn(struct work_struct *work)
{
    struct zg01_chain *c = container_of(work, struct zg01_chain, quiesce_work.work);
    struct zg01_dev *dev = c->dev;

    mutex_lock(&dev->state_mutex);
    if (!chain_consumers_running(dev, c))
        zg01_chain_stop(dev, c);
    if (!chain_consumers_running(dev, &dev->in_chain))
        zg01_chain_stop(dev, &dev->in_chain);
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
    atomic_set(&c->kill, 1);
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
    unsigned long flags;

    lockdep_assert_held(&dev->state_mutex);

    if (!c->allocated)
        return;
    if (atomic_read(&c->kill))
        return;                    /* already stopped or stopping */

    spin_lock_irqsave(&dev->lock, flags);
    atomic_set(&c->kill, 1);
    if (c == &dev->out_chain)
        zg01_feedback_reset(&dev->feedback);
    spin_unlock_irqrestore(&dev->lock, flags);
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
    unsigned long flags;

    if (!c->allocated)
        return -ENOMEM;

    /* Async-cancel a pending quiesce; if it wins the race it no-ops
     * (consumer is running again).  Must NOT be _sync here: the
     * quiesce handler takes state_mutex. */
    cancel_delayed_work(&c->quiesce_work);

    mutex_lock(&dev->state_mutex);

    /* Never submit against a device being torn down — disconnect
     * frees the URBs under this mutex after setting disconnecting. */
    if (atomic_read(&dev->disconnecting)) {
        mutex_unlock(&dev->state_mutex);
        return -ENODEV;
    }

    while (c->cleanup_pending) {
        mutex_unlock(&dev->state_mutex);
        flush_work(&c->cleanup_work);   /* also drains callbacks */
        mutex_lock(&dev->state_mutex);
        if (atomic_read(&dev->disconnecting) || !c->allocated) {
            mutex_unlock(&dev->state_mutex);
            return -ENODEV;
        }
        if (!c->cleanup_pending)
            continue;
    }

    /* A terminal URB completion can kill a chain from callback context.
     * Convert that state into the normal synchronous cleanup path before
     * a later START can submit or adopt any remaining URBs. */
    if (atomic_read(&c->kill) && atomic_read(&c->inflight) > 0) {
        c->cleanup_pending = true;
        for (i = 0; i < MAX_URBS; i++)
            if (c->urbs[i])
                usb_unlink_urb(c->urbs[i]);
        queue_work(zg01_cleanup_wq, &c->cleanup_work);
    }
    while (c->cleanup_pending) {
        mutex_unlock(&dev->state_mutex);
        flush_work(&c->cleanup_work);
        mutex_lock(&dev->state_mutex);
        if (atomic_read(&dev->disconnecting) || !c->allocated) {
            mutex_unlock(&dev->state_mutex);
            return -ENODEV;
        }
    }

    if (atomic_read(&dev->disconnecting) || !c->allocated ||
        !chain_consumers_running(dev, c)) {
        mutex_unlock(&dev->state_mutex);
        return -ECANCELED;
    }
    if (!atomic_read(&c->kill)) {
        mutex_unlock(&dev->state_mutex);
        return 0;                        /* adopt live chain */
    }

    spin_lock_irqsave(&dev->lock, flags);
    atomic_set(&c->kill, 0);
    if (c == &dev->out_chain) {
        zg01_feedback_reset(&dev->feedback);
        dev->have_last_plan = false;
        dev->feedback_started = false;
        dev->feedback_fault = false;
        dev->feedback_startup_urbs = 0;
        memset(c->completed_frames, 0, sizeof(c->completed_frames));
        for (i = 0; i < MAX_URBS; i++) {
            memset(c->bufs[i], 0, c->iso_pkts * c->iso_pkt_size);
            zg01_feedback_pending(&dev->feedback, i);
        }
        spin_unlock_irqrestore(&dev->lock, flags);
        mutex_unlock(&dev->state_mutex);
        return 0;
    }
    spin_unlock_irqrestore(&dev->lock, flags);
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
    /* URBs never handed to usb_submit_urb can never decrement
     * inflight via a completion — subtract them here, or the counter
     * sticks >0 and a later start "adopts" a dead chain forever. */
    atomic_sub(MAX_URBS - submitted, &c->inflight);
    spin_lock_irqsave(&dev->lock, flags);
    atomic_set(&c->kill, 1);
    spin_unlock_irqrestore(&dev->lock, flags);
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

/* Account every non-cancelled packet, before any PCM length/state filter.
 * Caller holds dev->lock. No printk or allocation in the completion path. */
static void zg01_usb_stats_account(struct zg01_usb_stats *s,
                                   const struct urb *urb, bool capture, u64 now)
{
    int i;

    s->completions++;
    if (urb->status == -ENOENT || urb->status == -ECONNRESET ||
        urb->status == -ESHUTDOWN) {
        s->cancelled++;
        return;
    }
    if (urb->status) {
        s->urb_errors++;
        s->last_error_ns = now;
    }

    for (i = 0; i < urb->number_of_packets; i++) {
        const struct usb_iso_packet_descriptor *p = &urb->iso_frame_desc[i];

        s->packets++;
        if (p->status <= 0 && p->status > -128)
            s->packet_status[-p->status]++;
        else
            s->unknown_status++;
        if (p->status) {
            s->last_error_ns = now;
            continue;
        }
        if (capture) {
            if (p->actual_length <= ISO_PKT_SIZE_IN)
                s->in_length[p->actual_length]++;
            else
                s->in_length_overflow++;
        } else if (p->actual_length != p->length) {
            s->out_length_mismatch++;
            s->last_error_ns = now;
        } else if (p->length >= 200 && p->length <= 280 && !(p->length % 40)) {
            s->out_frames[p->length / 40]++;
        }
    }
}

/* Retain startup and the packet on either side of a length/status change.
 * Freeze records at capacity; continue counting omitted selected records. */
static void zg01_in_trace_append(struct zg01_in_trace *t,
                                 const struct zg01_in_record *r)
{
    if (t->count && t->records[t->count - 1].seq == r->seq)
        return;
    if (t->count == ZG01_TRACE_RECORDS) {
        t->omitted++;
        return;
    }
    t->records[t->count++] = *r;
}

static void zg01_in_trace_account(struct zg01_in_trace *t,
                                  const struct urb *urb, u64 now)
{
    int i;

    if (urb->status) {
        t->have_previous = false;
        return;
    }
    for (i = 0; i < urb->number_of_packets; i++) {
        const struct usb_iso_packet_descriptor *p = &urb->iso_frame_desc[i];
        struct zg01_in_record r = {
            .seq = ++t->packets,
            .completion_ns = now,
            .length = p->actual_length,
            .status = p->status,
        };
        bool changed;

        /* Never read failed, short, or out-of-bounds packet data. */
        if (!p->status && p->actual_length >= sizeof(r.header) &&
            p->actual_length <= p->length && urb->transfer_buffer &&
            urb->transfer_buffer_length >= 0 &&
            p->offset <= (unsigned int)urb->transfer_buffer_length &&
            p->actual_length <= (unsigned int)urb->transfer_buffer_length - p->offset) {
            memcpy(r.header, (u8 *)urb->transfer_buffer + p->offset,
                   sizeof(r.header));
            r.header_valid = true;
        }
        changed = t->have_previous &&
                  (r.length != t->previous.length ||
                   r.status != t->previous.status);
        if (changed)
            zg01_in_trace_append(t, &t->previous);
        if (!t->have_previous || r.seq <= 4 || changed || r.status ||
            r.length != 108)
            zg01_in_trace_append(t, &r);
        t->previous = r;
        t->have_previous = true;
    }
}

static void zg01_in_trace_read(struct snd_info_entry *entry,
                               struct snd_info_buffer *buffer)
{
    struct zg01_dev *dev = entry->private_data;
    struct zg01_in_trace *t;
    unsigned long flags;
    unsigned int i;

    t = kmalloc(sizeof(*t), GFP_KERNEL);
    if (!t) {
        snd_iprintf(buffer, "snapshot allocation failed\n");
        return;
    }
    spin_lock_irqsave(&dev->lock, flags);
    *t = dev->in_trace;
    spin_unlock_irqrestore(&dev->lock, flags);

    snd_iprintf(buffer, "zg01_in_trace_v1\npackets %llu\nretained %u\nomitted %llu\n",
                t->packets, t->count, t->omitted);
    snd_iprintf(buffer, "# seq completion_ns length status header_hex\n");
    for (i = 0; i < t->count; i++) {
        const struct zg01_in_record *r = &t->records[i];

        snd_iprintf(buffer, "%llu %llu %u %d ", r->seq,
                    r->completion_ns, r->length, r->status);
        if (r->header_valid)
            snd_iprintf(buffer, "%8phN\n", r->header);
        else
            snd_iprintf(buffer, "-\n");
    }
    kfree(t);
}

static void zg01_usb_stats_print(struct snd_info_buffer *buffer,
                                 const struct zg01_usb_stats *s, bool capture)
{
    int i;

    snd_iprintf(buffer, "%s\n", capture ? "IN 0x81" : "OUT 0x01");
    snd_iprintf(buffer, "completions %llu\ncancelled %llu\nurb_errors %llu\n",
                s->completions, s->cancelled, s->urb_errors);
    snd_iprintf(buffer, "packets %llu\nunknown_status %llu\nlast_error_ns %llu\n",
                s->packets, s->unknown_status, s->last_error_ns);
    snd_iprintf(buffer, "feedback_valid %llu\nfeedback_invalid %llu\n"
                "feedback_starved %llu\nfeedback_overflow %llu\n"
                "feedback_submit_errors %llu\nplayback_waits %llu\n"
                "driver_xruns %llu\n",
                s->feedback_valid, s->feedback_invalid, s->feedback_starved,
                s->feedback_overflow, s->feedback_submit_errors, s->playback_waits,
                s->driver_xruns);
    for (i = 0; i < ARRAY_SIZE(s->packet_status); i++)
        if (s->packet_status[i])
            snd_iprintf(buffer, "packet_status %d %llu\n", -i,
                        s->packet_status[i]);
    if (capture) {
        for (i = 0; i < ARRAY_SIZE(s->in_length); i++)
            if (s->in_length[i])
                snd_iprintf(buffer, "in_length %d %llu\n", i, s->in_length[i]);
        snd_iprintf(buffer, "in_length_overflow %llu\n", s->in_length_overflow);
    } else {
        snd_iprintf(buffer, "out_length_mismatch %llu\n", s->out_length_mismatch);
        for (i = 5; i <= 7; i++)
            snd_iprintf(buffer, "out_frames %d %llu\n", i, s->out_frames[i]);
    }
}

static void zg01_usb_stats_read(struct snd_info_entry *entry,
                                struct snd_info_buffer *buffer)
{
    struct zg01_dev *dev = entry->private_data;
    struct zg01_usb_stats *snapshot;
    unsigned long flags;
    u64 now;

    /* Keep the snapshots off the kernel stack; format after releasing lock. */
    snapshot = kmalloc_array(2, sizeof(*snapshot), GFP_KERNEL);
    if (!snapshot) {
        snd_iprintf(buffer, "snapshot allocation failed\n");
        return;
    }
    spin_lock_irqsave(&dev->lock, flags);
    snapshot[0] = dev->out_chain.stats;
    snapshot[1] = dev->in_chain.stats;
    now = ktime_get_ns();
    spin_unlock_irqrestore(&dev->lock, flags);

    snd_iprintf(buffer, "zg01_usb_stats_v1\nsnapshot_ns %llu\n", now);
    zg01_usb_stats_print(buffer, &snapshot[0], false);
    zg01_usb_stats_print(buffer, &snapshot[1], true);
    kfree(snapshot);
}

/* Resubmit gate.  Returns true when the URB was resubmitted; on false
 * the caller must NOT touch the chain again (inflight already dropped). */
static bool chain_resubmit(struct zg01_chain *c, struct urb *urb)
{
    struct zg01_dev *dev = c->dev;
    int i, ret;
    bool terminal = urb->status == -ESHUTDOWN ||
                    urb->status == -ENOENT ||
                    urb->status == -ECONNRESET;

    if (atomic_read(&dev->disconnecting) || atomic_read(&c->kill) ||
        terminal) {
        if (terminal && !atomic_read(&dev->disconnecting) &&
            !atomic_read(&c->kill)) {
            atomic_set(&c->kill, 1);
            zg01_feedback_xrun(dev);
        }
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
        /* Surface the dead chain to ALSA as an xrun instead of
         * clicking silently forever.  Queuing work from softirq is
         * safe; the handlers check substream state themselves. */
        c->stats.feedback_submit_errors++;
        zg01_feedback_xrun(dev);
        return false;
    }
    return true;
}

/* Snapshot and all ring access are serialized against STOP/hw_free. */
struct out_consumer {
    struct zg01_stream *s;
    struct snd_pcm_runtime *rt;
    unsigned int available;
    bool active;
};

static void snap_consumer(struct zg01_dev *dev, enum zg01_stream_id id,
                          struct out_consumer *oc)
{
    struct zg01_stream *s = &dev->streams[id];
    struct snd_pcm_substream *sub = s->substream;

    memset(oc, 0, sizeof(*oc));
    oc->s = s;
    if (!s->enabled || !sub || !sub->runtime || !sub->runtime->dma_area ||
        !snd_pcm_running(sub))
        return;
    oc->rt = sub->runtime;
    oc->active = true;
    oc->available = zg01_playback_available(READ_ONCE(oc->rt->control->appl_ptr),
                         s->queued_ptr, oc->rt->boundary, oc->rt->buffer_size);
}

static void zg01_feedback_xrun(struct zg01_dev *dev)
{
    int i;

    dev->feedback_fault = true;
    dev->out_chain.stats.driver_xruns++;
    for (i = 0; i < ZG01_N_STREAMS; i++) {
        if (dev->streams[i].enabled && !dev->streams[i].xrun_generation) {
            dev->streams[i].xrun_generation = dev->streams[i].generation;
            queue_work(zg01_period_wq, &dev->streams[i].xrun_work);
        }
    }
}

/* dev->lock covers submit as well as kill publication: IN callbacks must
 * never submit OUT after its stop path has begun draining. Maximum two
 * submitted OUT URBs bounds copy-ahead below the smallest playback ring. */
static void zg01_feedback_pump(struct zg01_dev *dev)
{
    struct zg01_chain *c = &dev->out_chain;
    struct zg01_feedback_queue *q = &dev->feedback;
    struct zg01_feedback_plan plan;
    struct out_consumer oc[2];
    unsigned int limit[2];
    unsigned int id, total, used[2], i, f, n;
    struct urb *urb;
    int ret;

    if (dev->feedback_fault)
        return;
    if (!dev->feedback_started) {
        if (q->plans < 2)
            return;
        dev->feedback_started = true;
    }
    while (!atomic_read(&dev->disconnecting) && !atomic_read(&c->kill) &&
           atomic_read(&c->inflight) < 2) {
        bool gap_fallback = false;

        if (q->plans && q->pending) {
            total = 0;
            for (i = 0; i < ISO_PKTS_OUT; i++)
                total += q->plan[q->plan_head].frames[i];
        } else if (dev->have_last_plan && dev->feedback_started) {
            /* Plan gap: keep OUT cadence on the last measured framing.
             * A stalled hw_ptr makes ALSA's in_interrupt jiffies heuristic
             * assume a ring wrap and fabricate a full-buffer hw_ptr jump,
             * which lands as a false XRUN.  The nominal clock is at most
             * ~21 ppm off until the next plan resyncs it. */
            total = 0;
            for (i = 0; i < ISO_PKTS_OUT; i++)
                total += dev->last_plan.frames[i];
            gap_fallback = true;
        } else {
            break;
        }
        if (gap_fallback)
            c->stats.feedback_starved++;
        snap_consumer(dev, ZG01_GAME, &oc[0]);
        snap_consumer(dev, ZG01_VOICE_OUT, &oc[1]);
        for (n = 0; n < 2; n++) {
            struct zg01_stream *s = oc[n].s;

            /* START precedes ALSA's PREPARED -> RUNNING transition.  A
             * starting consumer contributes silence for now; it must not
             * stall the already-running sibling's submissions (a drain
             * and burst catch-up drives hw_ptr into appl_ptr). */
            if (s->enabled && !oc[n].active) {
                s->wait_since_ns = 0;
                limit[n] = 0;
                continue;
            }
            if (!oc[n].active ||
                oc[n].rt->status->state == SNDRV_PCM_STATE_DRAINING) {
                s->wait_since_ns = 0;
                limit[n] = oc[n].available;   /* drain consumes all */
                continue;
            }
            /* RUNNING: keep one URB of guard frames unconsumed so a
             * completion can never land exactly on appl_ptr before the
             * userspace write does (per-URB retirement vs per-period
             * refill makes that race routine under load).  DRAINING
             * must consume everything. */
            limit[n] = oc[n].available > ZG01_PLAY_GUARD
                     ? oc[n].available - ZG01_PLAY_GUARD : 0;
            if (oc[n].available >= total) {
                s->wait_since_ns = 0;
                continue;
            }
            /* Partial fill: the copy loop caps used[n] at limit[n] and
             * pads the rest with silence.  Stalling the whole pump here
             * deadlocks userspace wakeups — aplay refills on
             * period_elapsed, which only fires after an OUT completion
             * retires frames. */
            if (limit[n] > 0) {
                c->stats.playback_waits++;
                s->wait_since_ns = 0;
                continue;
            }
            /* At or below the guard: true starvation, but only if
             * nothing is still in flight to wake the stream. */
            c->stats.playback_waits++;
            if (s->queued_pos != s->pcm_pos)
                return; /* in-flight completion will pump again */
            /* Period notification is deferred; give userspace two URB
             * intervals to refill before declaring a true starvation. */
            if (!s->wait_since_ns)
                s->wait_since_ns = ktime_get_ns();
            if (ktime_get_ns() - s->wait_since_ns < 8000000)
                return;
            zg01_feedback_xrun(dev);
            return;
        }
        if (gap_fallback) {
            plan = dev->last_plan;
            if (!zg01_feedback_pending_take(q, &id))
                return;
        } else if (!zg01_feedback_take(q, &plan, &id)) {
            return;
        }
        urb = c->urbs[id];
        memset(urb->transfer_buffer, 0, c->iso_pkts * c->iso_pkt_size);
        used[0] = used[1] = 0;
        for (i = 0; i < ISO_PKTS_OUT; i++) {
            u8 *pkt = urb->transfer_buffer + i * ISO_PKT_SIZE_OUT;

            urb->iso_frame_desc[i].length = plan.frames[i] * 40;
            urb->iso_frame_desc[i].actual_length = 0;
            urb->iso_frame_desc[i].status = 0;
            for (f = 0; f < plan.frames[i]; f++) {
                for (n = 0; n < 2; n++) {
                    unsigned int off;
                    struct zg01_stream *s = oc[n].s;

                    if (!oc[n].active || used[n] >= limit[n])
                        continue;
                    off = ((s->queued_pos + used[n]) % oc[n].rt->buffer_size) * 8;
                    memcpy(pkt + f * 40 + (n == 0 ? 8 : 0),
                           oc[n].rt->dma_area + off, 8);
                    used[n]++;
                }
            }
        }
        for (n = 0; n < 2; n++) {
            c->completed_frames[id][n] = used[n];
            c->generation[id][n] = oc[n].s->generation;
        }
        atomic_inc(&c->inflight);
        ret = usb_submit_urb(urb, GFP_ATOMIC);
        if (ret) {
            atomic_dec(&c->inflight);
            c->stats.feedback_submit_errors++;
            zg01_feedback_pending(q, id);
            zg01_feedback_xrun(dev);
            return;
        }
        for (n = 0; n < 2; n++) {
            if (!used[n])
                continue;
            oc[n].s->queued_pos += used[n];
            oc[n].s->queued_ptr = (oc[n].s->queued_ptr + used[n]) % oc[n].rt->boundary;
        }
    }
}

static void zg01_iso_out(struct urb *urb)
{
    struct zg01_chain *c = urb->context;
    struct zg01_dev *dev = c->dev;
    unsigned long flags;
    unsigned int id, n;
    bool terminal = urb->status == -ENOENT || urb->status == -ECONNRESET ||
                    urb->status == -ESHUTDOWN;

    spin_lock_irqsave(&dev->lock, flags);
    zg01_usb_stats_account(&c->stats, urb, false, ktime_get_ns());
    for (id = 0; id < MAX_URBS && c->urbs[id] != urb; id++)
        ;
    atomic_dec(&c->inflight);
    if (terminal && !atomic_read(&c->kill) && !atomic_read(&dev->disconnecting)) {
        atomic_set(&c->kill, 1);
        zg01_feedback_xrun(dev);
    }
    if (id == MAX_URBS || terminal || atomic_read(&c->kill) ||
        atomic_read(&dev->disconnecting))
        goto unlock;
    for (n = 0; n < 2; n++) {
        struct zg01_stream *s = &dev->streams[n];
        unsigned int frames = c->completed_frames[id][n];

        /* Retire submitted samples once, including lost USB packets;
         * never replay a partly successful URB. USB errors remain counted. */
        if (frames && s->enabled && s->generation == c->generation[id][n]) {
            s->pcm_pos += frames;
            /* Also wakes a drain shorter than period_size. */
            queue_work(zg01_period_wq, &s->period_work);
        }
        c->completed_frames[id][n] = 0;
    }
    zg01_feedback_pending(&dev->feedback, id);
    if (!dev->feedback.plans)
        c->stats.feedback_starved++;
    zg01_feedback_pump(dev);
unlock:
    spin_unlock_irqrestore(&dev->lock, flags);
}

/* IN is a shared clock source, independent of whether capture is open.
 * Header word 1 is payload bytes; word 0 is an observed packet sequence.
 * Only validated 5/6/7-frame packets supply capture or feedback. */
static void zg01_iso_in(struct urb *urb)
{
    struct zg01_chain *c = urb->context;
    struct zg01_dev *dev = c->dev;
    struct zg01_stream *s = &dev->streams[ZG01_VOICE_IN];
    struct snd_pcm_substream *sub;
    struct snd_pcm_runtime *rt;
    struct zg01_feedback_plan plan = {0};
    unsigned long flags;
    unsigned int old_pos, i, f;
    bool valid = urb->status == 0 && urb->number_of_packets == ISO_PKTS_IN;

    spin_lock_irqsave(&dev->lock, flags);
    zg01_usb_stats_account(&c->stats, urb, true, ktime_get_ns());
    zg01_in_trace_account(&dev->in_trace, urb, ktime_get_ns());
    sub = s->substream;
    rt = (s->enabled && sub && sub->runtime && sub->runtime->dma_area &&
          snd_pcm_running(sub)) ? sub->runtime : NULL;
    old_pos = s->pcm_pos;
    if (!atomic_read(&c->kill) && !atomic_read(&dev->disconnecting)) {
        for (i = 0; i < urb->number_of_packets && i < ISO_PKTS_IN; i++) {
            const struct usb_iso_packet_descriptor *p = &urb->iso_frame_desc[i];
            unsigned int frames = 0;

            if (!urb->status && urb->transfer_buffer_length >= 0)
                frames = zg01_packet_frames(urb->transfer_buffer,
                            urb->transfer_buffer_length, p->offset,
                            p->actual_length, p->length, p->status);
            if (!frames) {
                c->stats.feedback_invalid++;
                valid = false;
                continue;
            }
            c->stats.feedback_valid++;
            plan.frames[i] = frames;
            if (!rt)
                continue;
            for (f = 0; f < frames; f++) {
                unsigned int off = (s->pcm_pos % rt->buffer_size) * 8;

                memcpy(rt->dma_area + off,
                       urb->transfer_buffer + p->offset + 8 + f * 16, 8);
                s->pcm_pos++;
            }
        }
        if (rt && s->pcm_pos != old_pos)
            queue_work(zg01_period_wq, &s->period_work);
        if (!atomic_read(&dev->out_chain.kill) &&
            (dev->feedback.pending || atomic_read(&dev->out_chain.inflight)) &&
            !dev->feedback_fault) {
            if (!valid && (dev->feedback_started ||
                          ++dev->feedback_startup_urbs >= MAX_URBS))
                zg01_feedback_xrun(dev);
        }
        if (valid && !dev->feedback_fault && !atomic_read(&dev->out_chain.kill) &&
            (dev->feedback.pending || atomic_read(&dev->out_chain.inflight))) {
            dev->last_plan = plan;
            dev->have_last_plan = true;
            if (!zg01_feedback_push(&dev->feedback, &plan)) {
                c->stats.feedback_overflow++;
                zg01_feedback_xrun(dev);
            }
        }
        zg01_feedback_pump(dev);
    }
    chain_resubmit(c, urb);
    spin_unlock_irqrestore(&dev->lock, flags);
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

    runtime->hw.info = SNDRV_PCM_INFO_MMAP | SNDRV_PCM_INFO_MMAP_VALID |
                       SNDRV_PCM_INFO_INTERLEAVED |
                       SNDRV_PCM_INFO_BLOCK_TRANSFER | SNDRV_PCM_INFO_BATCH;
    runtime->hw.formats = SNDRV_PCM_FMTBIT_S32_LE;
    runtime->hw.channels_min = 2;
    runtime->hw.channels_max = 2;
    runtime->hw.periods_min = 2;
    runtime->hw.periods_max = 64;

    if (s->direction == SNDRV_PCM_STREAM_CAPTURE) {
        runtime->hw.rates = SNDRV_PCM_RATE_48000;
        runtime->hw.rate_min = 48000;
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
 * can still hold the old substream.
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
    spin_lock_irqsave(&dev->lock, flags);
    s->enabled = false;
    s->generation++;
    s->xrun_generation = 0;
    s->substream = NULL;
    spin_unlock_irqrestore(&dev->lock, flags);
    if (!chain_consumers_running(dev, c))
        zg01_chain_stop(dev, c);
    if (!chain_consumers_running(dev, &dev->in_chain))
        zg01_chain_stop(dev, &dev->in_chain);
    mutex_unlock(&dev->state_mutex);

    flush_work(&dev->out_chain.cleanup_work);
    flush_work(&dev->in_chain.cleanup_work);
    cancel_delayed_work_sync(&dev->out_chain.quiesce_work);
    cancel_delayed_work_sync(&dev->in_chain.quiesce_work);
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
    if (params_rate(hw_params) != 48000)
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
    spin_lock_irqsave(&dev->lock, flags);
    s->enabled = false;
    s->generation++;
    s->xrun_generation = 0;
    s->substream = NULL;
    spin_unlock_irqrestore(&dev->lock, flags);
    if (!chain_consumers_running(dev, c))
        zg01_chain_stop(dev, c);
    if (!chain_consumers_running(dev, &dev->in_chain))
        zg01_chain_stop(dev, &dev->in_chain);
    mutex_unlock(&dev->state_mutex);

    flush_work(&dev->out_chain.cleanup_work);
    flush_work(&dev->in_chain.cleanup_work);
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
     * check atomic against triggers on the sibling PCMs.
     *
     * The clock is DEVICE-WIDE (one UAC2 clock source shared by
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

    /* Playback also owns IN for implicit feedback. Never reset a live
     * interface when capture joins/leaves or either playback PCM joins. */
    if (atomic_read(&dev->in_chain.inflight) == 0)
        usb_set_interface(dev->udev, 2, 1);
    ret = zg01_chain_alloc(dev, &dev->in_chain, ZG01_EP_IN, 2,
                           ISO_PKTS_IN, ISO_PKT_SIZE_IN, false);
    if (!ret && s->direction != SNDRV_PCM_STREAM_CAPTURE) {
        if (!dev->out_chain.allocated || atomic_read(&dev->out_chain.kill))
            usb_set_interface(dev->udev, 1, 1);
        ret = zg01_chain_alloc(dev, &dev->out_chain, ZG01_EP_OUT, 1,
                               ISO_PKTS_OUT, ISO_PKT_SIZE_OUT, true);
    }
    if (ret) {
        mutex_unlock(&dev->state_mutex);
        return ret;
    }

    /*
     * Publish the substream and reset the position under dev->lock.
     * The callback advances this position for every frame submitted to
     * the endpoint. ALSA compares it with appl_ptr to detect underruns.
     */
    spin_lock_irqsave(&dev->lock, flags);
    s->substream = substream;
    s->pcm_pos = 0;
    s->queued_pos = 0;
    s->generation++;
    s->enabled = false;
    s->xrun_generation = 0;
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
    unsigned long flags;

    mutex_lock(&dev->state_mutex);

    switch (cmd) {
    case SNDRV_PCM_TRIGGER_START:
        if (!c->allocated) {
            mutex_unlock(&dev->state_mutex);
            return -ENOMEM;
        }
        s->running = true;
        spin_lock_irqsave(&dev->lock, flags);
        s->enabled = true;
        s->generation++;
        s->xrun_generation = 0;
        s->queued_pos = s->pcm_pos;
        s->queued_ptr = substream->runtime->status->hw_ptr;
        s->wait_since_ns = 0;
        spin_unlock_irqrestore(&dev->lock, flags);
        mutex_unlock(&dev->state_mutex);

        /* May sleep (cleanup drain); state re-checked inside. */
        ret = zg01_chain_start(dev, c);
        if (!ret && c == &dev->out_chain)
            ret = zg01_chain_start(dev, &dev->in_chain);
        if (ret) {
            mutex_lock(&dev->state_mutex);
            s->running = false;
            spin_lock_irqsave(&dev->lock, flags);
            s->enabled = false;
            s->xrun_generation = 0;
            spin_unlock_irqrestore(&dev->lock, flags);
            if (!chain_consumers_running(dev, &dev->out_chain))
                zg01_chain_stop(dev, &dev->out_chain);
            if (!chain_consumers_running(dev, &dev->in_chain))
                zg01_chain_stop(dev, &dev->in_chain);
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
        spin_lock_irqsave(&dev->lock, flags);
        s->enabled = false;
        s->generation++;
        s->xrun_generation = 0;
        spin_unlock_irqrestore(&dev->lock, flags);
        if (rapid) {
            /* Keep the chain cycling (silence); the quiesce timer
             * stops it if no START follows. */
            mod_delayed_work(zg01_cleanup_wq, &c->quiesce_work,
                             ZG01_QUIESCE_DELAY);
            mutex_unlock(&dev->state_mutex);
            return 0;
        }
        if (!chain_consumers_running(dev, c))
            zg01_chain_stop(dev, c);
        if (!chain_consumers_running(dev, &dev->in_chain))
            zg01_chain_stop(dev, &dev->in_chain);
        mutex_unlock(&dev->state_mutex);
        return 0;

    case SNDRV_PCM_TRIGGER_SUSPEND:
        s->running = false;
        spin_lock_irqsave(&dev->lock, flags);
        s->enabled = false;
        s->generation++;
        s->xrun_generation = 0;
        spin_unlock_irqrestore(&dev->lock, flags);
        if (!chain_consumers_running(dev, c))
            zg01_chain_stop(dev, c);
        if (!chain_consumers_running(dev, &dev->in_chain))
            zg01_chain_stop(dev, &dev->in_chain);
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

static int zg01_pcm_ack(struct snd_pcm_substream *substream)
{
    struct zg01_dev *dev = sub_to_stream(substream)->dev;
    unsigned long flags;

    spin_lock_irqsave(&dev->lock, flags);
    zg01_feedback_pump(dev);
    spin_unlock_irqrestore(&dev->lock, flags);
    return 0;
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
    .ack = zg01_pcm_ack,
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

    ret = zg01_new_pcm(dev, ZG01_GAME, "ZG01 Game Out", 1, 0,
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

        /* Managed buffer at FULL size: the prealloc floor is also the
         * hw_params ceiling (no realloc path), and an 8 KB prealloc
         * clamps userspace to a 1020-frame ring.  On that ring ALSA's
         * stale-pointer threshold (~10.6 ms) sits below routine workqueue
         * latency, which fabricates hw_ptr wraps = false XRUNs. */
        snd_pcm_set_managed_buffer_all(dev->pcm_instances[i],
                                       SNDRV_DMA_TYPE_CONTINUOUS, NULL,
                                       max, max);
    }

    ret = snd_card_ro_proc_new(dev->card, "usb_stats", dev,
                               zg01_usb_stats_read);
    if (ret)
        return ret;
    return snd_card_ro_proc_new(dev->card, "in_trace", dev,
                                zg01_in_trace_read);
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
     * stop already quiesced.
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
