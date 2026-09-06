#ifndef ZG01_H
#define ZG01_H

#include <linux/usb.h>
#include <linux/spinlock.h>
#include <linux/workqueue.h>
#include <sound/core.h>
#include <sound/pcm.h>
#include "zg01_feedback.h"

#define VENDOR_ID_YAMAHA 0x0499
#define PRODUCT_ID_ZG01  0x1513

/*
 * USB layout (from Windows capture analysis):
 *   Interface 1, Alt 1: EP 0x01 OUT, isochronous
 *       - shared by Game and Voice Out playback
 *       - 240-byte packet = 6 frames of 40 bytes
 *       - frame = Voice_L(4) Voice_R(4) Game_L(4) Game_R(4) + 24 pad
 *   Interface 2, Alt 1: EP 0x81 IN, isochronous
 *       - Voice In capture, 108-byte packets
 *         (8-byte header + 6 frames x 16 bytes + 4-byte trailer)
 */
#define ZG01_EP_OUT          0x01
#define ZG01_EP_IN           0x81

#define ISO_PKTS_OUT         32      /* 32 microframes = 4ms per URB */
#define ISO_PKT_SIZE_OUT     280     /* up to seven 40-byte frames */
#define ISO_PKTS_IN          32
#define ISO_PKT_SIZE_IN      124     /* header + up to seven frames + trailer */
#define MAX_ISO_PACKET_SIZE  8192

#define MAX_URBS             16      /* 64ms of buffering */

/* PCM device numbers on the single card */
#define ZG01_PCM_GAME        0
#define ZG01_PCM_VOICE_OUT   1
#define ZG01_PCM_VOICE_IN    2

enum zg01_stream_id {
    ZG01_GAME = 0,
    ZG01_VOICE_OUT,
    ZG01_VOICE_IN,
    ZG01_N_STREAMS,
};

/*
 * Per-PCM-device stream state.  Game and Voice Out are two independent
 * ALSA playback PCMs whose samples are mixed into the packets of the
 * single EP 0x01 URB chain; the ZG01 firmware performs the analog mix.
 * PipeWire exposes one node per PCM device, so the single-card topology
 * keeps three independent sinks/sources.
 */
struct zg01_stream {
    struct zg01_dev *dev;
    unsigned int pcm_device;                 /* ALSA pcm device number */
    int direction;                           /* SNDRV_PCM_STREAM_* */

    /* Callback-shared state, protected by dev->lock: */
    struct snd_pcm_substream *substream;
    unsigned int pcm_pos;                    /* absolute frame counter */
    unsigned int queued_pos;                 /* copied, not yet completed */
    snd_pcm_uframes_t queued_ptr;             /* ALSA boundary epoch */
    unsigned int generation;                 /* reject pre-STOP completions */
    unsigned int xrun_generation;            /* deferred failure epoch */
    bool enabled;                            /* trigger gate under dev->lock */
    u64 wait_since_ns;                       /* bounded application refill */

    /* PCM-op state, protected by dev->state_mutex: */
    bool opened;                             /* open() .. close() */
    bool initialized;                        /* device init has run */
    bool running;                            /* TRIGGER_START .. STOP */

    /* Rapid START/STOP suppression (PipeWire reconfiguration bursts) */
    unsigned long last_trigger_jiffies;
    unsigned int trigger_count;

    struct work_struct period_work;          /* deferred elapsed */
    struct work_struct xrun_work;            /* deferred stop_xrun */
};

/*
 * One URB chain per isochronous endpoint.  The out chain is a shared
 * resource: it runs while EITHER Game or Voice Out needs it, and the
 * callback fills each consumer's slot pair from its own PCM ring
 * (silence for a consumer that is not running).  No handoff, no
 * promotion, no piggyback flag set.
 *
 * Transport state is separate from allocation and PCM consumer demand.
 * dev->lock protects transitions and the submission gate. Process paths
 * take state_mutex before dev->lock; callbacks take only dev->lock.
 * STOPPED permits reset, STARTING permits initial IN submits/resubmits,
 * RUNNING includes OUT waiting for feedback with no URBs submitted.
 * Only cleanup may move DRAINING to STOPPED, after usb_kill_urb() has
 * joined every callback of that chain. inflight is accounting, not proof
 * that cleanup has finished.
 */
enum zg01_chain_state {
    ZG01_CHAIN_STOPPED,
    ZG01_CHAIN_STARTING,
    ZG01_CHAIN_RUNNING,
    ZG01_CHAIN_DRAINING,
};
/* Diagnostic counters, protected by dev->lock. Cumulative until unplug. */
struct zg01_usb_stats {
    u64 completions;
    u64 cancelled;
    u64 urb_errors;
    u64 packets;
    u64 packet_status[128]; /* index = -errno, zero = success */
    u64 unknown_status;
    u64 in_length[ISO_PKT_SIZE_IN + 1]; /* successful packets only */
    u64 in_length_overflow;
    u64 out_length_mismatch;
    u64 out_frames[8];                       /* successful complete packets */
    u64 last_error_ns;
    u64 feedback_valid;
    u64 feedback_invalid;
    u64 feedback_starved;
    u64 feedback_overflow;
    u64 feedback_submit_errors;
    u64 playback_waits;
    u64 driver_xruns;                       /* zg01_feedback_xrun calls */
};

struct zg01_chain {
    struct zg01_dev *dev;
    unsigned int endpoint;
    unsigned int interface_num;               /* USB interface to alt 1 */
    unsigned int iso_pkts;
    unsigned int iso_pkt_size;

    struct zg01_usb_stats stats;

    struct urb *urbs[MAX_URBS];
    unsigned char *bufs[MAX_URBS];
    unsigned int completed_frames[MAX_URBS][2];
    unsigned int generation[MAX_URBS][2];

    bool allocated;                           /* state_mutex */
    enum zg01_chain_state state;              /* dev->lock; READ_ONCE snapshots */
    atomic_t inflight;                        /* URB accounting, not a callback join */

    struct work_struct cleanup_work;          /* sole DRAINING -> STOPPED owner */
    struct delayed_work quiesce_work;         /* stop idle suppressed chain */
};

#define ZG01_TRACE_RECORDS 256

/* Bounded IN header trace: a ring of the MOST RECENT change-points.
 * Header bytes + length/status of every successful packet; no payload.
 * Protected by dev->lock. */
struct zg01_in_record {
    u64 seq;
    u64 completion_ns;
    unsigned int length;
    int status;
    unsigned char header[8];
    bool header_valid;
};

struct zg01_in_trace {
    u64 packets;
    u64 omitted;
    unsigned int count;               /* valid records in ring order */
    unsigned int next;                /* next write slot (ring head) */
    bool have_previous;
    struct zg01_in_record previous;
    struct zg01_in_record records[ZG01_TRACE_RECORDS];
};

struct zg01_dev {
    struct usb_device *udev;
    struct snd_card *card;
    struct usb_interface *interface;          /* interface 1 (anchor) */

    struct zg01_stream streams[ZG01_N_STREAMS];
    struct zg01_chain out_chain;              /* EP 0x01: game + voice out */
    struct zg01_chain in_chain;               /* EP 0x81: voice in */
    struct snd_pcm *pcm_instances[ZG01_N_STREAMS];
    struct zg01_in_trace in_trace;
    struct zg01_feedback_queue feedback;
    struct zg01_feedback_plan last_plan;     /* dev->lock: gap fallback */
    bool have_last_plan;                     /* dev->lock */
    bool feedback_started;                   /* dev->lock: priming complete */
    bool feedback_fault;                     /* latched until drained restart */
    unsigned int feedback_gap_urbs;          /* consecutive fallback URBs */
    unsigned int feedback_startup_urbs;       /* bounded no-feedback startup */

#define ZG01_GAP_FALLBACK_MAX_URBS 125    /* ~500 ms at ~4 ms per URB */

    /*
     * dev->lock guards the callback-shared fields (substream pointers,
     * pcm_pos).  Every dereference of substream/runtime/
     * dma_area in the URB callbacks happens inside this lock, and every
     * clear of those pointers also happens inside it. Sibling transport
     * may remain active after a stream detaches. One device, one lock:
     * the cross-card mismatch window of the previous multi-card design
     * is gone.
     */
    spinlock_t lock;

    /*
     * state_mutex serializes all sleeping PCM operations (open, close,
     * hw_params, hw_free, prepare, trigger) and interface altsettings.
     * Callback fault transitions use dev->lock. trigger may sleep (the
     * PCMs are nonatomic), so check-then-act sequences like "skip the
     * Magic Sequence if anything streams" are now atomic against
     * concurrent triggers on the sibling PCMs.
     */
    struct mutex state_mutex;

    bool device_initialized;                  /* vendor handshake + rate */
    unsigned int current_rate;

    atomic_t disconnecting;                   /* URB resubmission off */
    atomic_t disconnected;                    /* teardown-once latch */

    unsigned long last_open_jiffies;          /* log rate limiting */
    unsigned int open_count;
};

/* Private workqueues — defined in zg01_usb.c */
extern struct workqueue_struct *zg01_cleanup_wq;
extern struct workqueue_struct *zg01_period_wq;

/* zg01_pcm.c */
int zg01_create_pcm_devices(struct zg01_dev *dev);
int zg01_set_rate(struct zg01_dev *dev, int rate);
void zg01_stop_all_chains(struct zg01_dev *dev);
void zg01_drain_all_chains(struct zg01_dev *dev);
void zg01_suspend_pcm(struct zg01_dev *dev);
void zg01_pm_reset_streams(struct zg01_dev *dev);
void zg01_period_work_fn(struct work_struct *work);
void zg01_xrun_work_fn(struct work_struct *work);
void zg01_chain_cleanup_fn(struct work_struct *work);
void zg01_chain_quiesce_fn(struct work_struct *work);

/* zg01_control.c */
int zg01_init_control(struct zg01_dev *dev);

/* zg01_usb_discovery.c (best-effort, debug only) */
int zg01_discover_usb_config(struct zg01_dev *dev);

#endif /* ZG01_H */
