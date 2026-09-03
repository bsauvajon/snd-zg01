#ifndef ZG01_H
#define ZG01_H

#include <linux/usb.h>
#include <linux/spinlock.h>
#include <sound/core.h>
#include <sound/pcm.h>

#define VENDOR_ID_YAMAHA 0x0499
#define PRODUCT_ID_ZG01  0x1513
/* Channel type constants */
#define CHANNEL_TYPE_GAME       0
#define CHANNEL_TYPE_VOICE_IN   1
#define CHANNEL_TYPE_VOICE_OUT  2


/* Audio streaming parameters based on actual USB descriptor analysis */
#define ISO_PKTS_GAME   32        /* 32 microframes = 4ms buffer per URB to match Windows driver */
#define ISO_PKTS_VOICE  32        /* 32 microframes = 4ms buffer per URB */
#define ISO_PKT_SIZE_GAME  240    /* 240 bytes per microframe as seen in Windows capture */
#define ISO_PKT_SIZE_VOICE 124    /* Actual max packet size for voice input (alloc size) */
#define MAX_ISO_PACKET_SIZE 8192  /* Maximum size for isochronous packet sanity checks */

/* USB endpoints from actual device analysis */
#define ZG01_EP_GAME_OUT   0x01   /* Game audio output endpoint (Interface 1, Alt 1) */
#define ZG01_EP_VOICE_IN   0x81   /* Voice audio input endpoint (Interface 2, Alt 1) */

#include "zg01_pcm.h"
#include "zg01_control.h"

/* Multi-URB streaming for stable isochronous transfers */
#define MAX_URBS_PER_CHANNEL 16   /* Optimal buffering: 64ms reduces clicks to ~2.17% */

struct zg01_dev {
    struct usb_device *udev;
    struct snd_card *card;
    struct usb_interface *interface;

    struct zg01_pcm pcm;
    struct zg01_control control;

    /* Support for dual audio channels */
    struct snd_pcm_substream *substream_game;
    struct snd_pcm_substream *substream_voice;
    struct snd_pcm_substream *substream_voice_out;
    
    /* Game channel (high bandwidth) - multiple URBs for stability */
    struct urb *iso_urbs_game[MAX_URBS_PER_CHANNEL];
    unsigned char *iso_buffers_game[MAX_URBS_PER_CHANNEL];
    dma_addr_t iso_dmas_game[MAX_URBS_PER_CHANNEL];
    atomic_t active_urbs_game;
    
    /* Voice channel (low bandwidth) - multiple URBs for stability */  
    struct urb *iso_urbs_voice[MAX_URBS_PER_CHANNEL];
    unsigned char *iso_buffers_voice[MAX_URBS_PER_CHANNEL];
    dma_addr_t iso_dmas_voice[MAX_URBS_PER_CHANNEL];
    atomic_t active_urbs_voice;
    
    /* Voice output channel (playback to voice output) - multiple URBs for stability */
    struct urb *iso_urbs_voice_out[MAX_URBS_PER_CHANNEL];
    unsigned char *iso_buffers_voice_out[MAX_URBS_PER_CHANNEL];
    dma_addr_t iso_dmas_voice_out[MAX_URBS_PER_CHANNEL];
    atomic_t active_urbs_voice_out;
    
    spinlock_t lock;
    struct mutex pcm_mutex; /* Protect concurrent PCM operations */
    unsigned int pcm_pos_game;
    unsigned int pcm_pos_voice;
    unsigned int pcm_pos_voice_out;
    
    /* Channel type identifier (0=game, 1=voice_in/capture, 2=voice_out/playback) */
    int channel_type;
    
    /* State tracking */
    bool game_channel_active;
    bool voice_channel_active;
    bool voice_out_channel_active;
    bool game_initialized;        /* Track if game channel has been initialized */
    bool voice_initialized;       /* Track if voice channel has been initialized */
    bool voice_out_initialized;   /* Track if voice output channel has been initialized */
    unsigned int current_rate;      /* Current sample rate (44100 or 48000) */
    
    bool cleanup_in_progress_game;
    bool cleanup_in_progress_voice;
    bool cleanup_in_progress_voice_out;
    atomic_t disconnecting;           /* Set when USB disconnect starts, prevents URB resubmission */

    /* Embedded work structs for URB cleanup - prevents GFP_ATOMIC allocation failures */
    struct work_struct cleanup_work_game;
    struct work_struct cleanup_work_voice;
    struct work_struct cleanup_work_voice_out;

    /* Deferred period-elapsed notification (PCM is nonatomic: snd_pcm_period_elapsed
     * acquires a sleeping mutex, unsafe to call from softirq/URB callback context) */
    struct work_struct period_work_game;
    struct work_struct period_work_voice;
    struct work_struct period_work_voice_out;
    unsigned long last_trigger_jiffies; /* jiffies at last TRIGGER_START */
    unsigned int trigger_count;          /* START/STOP cycles within 100ms window */

    /*
     * EP 0x01 concurrent output mixing
     *
     * Game and Voice Out share the same USB endpoint (EP 0x01 OUT on Interface 1).
     * Having two independent URB chains on the same ISO endpoint causes bandwidth
     * contention and audio saccades.  Instead, when both channels are active:
     *   - Voice Out "piggybacks" onto the Game URB stream
     *   - Game callback reads Voice Out PCM data and mixes it into each packet
     *   - Voice Out submits no URBs of its own
     *
     * Fields in game_dev:
     *   voice_out_mixing   — 1 while Voice Out is piggybacking (atomic, used in IRQ)
     *   vo_mix_dev         — pointer to voice_out_dev, valid while mixing == 1
     *
     * Fields in voice_out_dev:
     *   voice_out_piggybacking — true  = no own URBs, relying on game stream
     *   piggyback_host         — back-pointer to game_dev while piggybacking
     */
    atomic_t voice_out_mixing;    /* game_dev field: 1 = VO piggybacking */
    struct zg01_dev *vo_mix_dev;  /* game_dev field: pointer to voice_out_dev */
    bool voice_out_piggybacking;  /* voice_out_dev field */
    struct zg01_dev *piggyback_host; /* voice_out_dev field: pointer to game_dev */

    /*
     * Shared EP 0x01 chain (game_dev fields).  When Game stops while
     * Voice Out piggybacks, the chain is NOT killed (the old fix
     * promoted Voice Out to its own URBs here): the callback silences
     * the Game slots and keeps mixing Voice Out.  Transitions are
     * serialized by chain_mutex (game_dev):
     *   Game STOP  with VO riding  -> chain_keepalive = true, chain runs
     *   VO detach  with keepalive  -> VO stops the orphaned chain
     *   Game START with keepalive  -> keepalive = false, Game re-owns it
     * Game hw_free while keepalive defers freeing the chain; the VO
     * detach teardown kills the URBs, which stay allocated for reuse
     * by the next Game prepare (disconnect frees them for real).
     */
    bool chain_keepalive;
    struct mutex chain_mutex;

    /*
     * Piggyback appl_ptr clamp (voice_out_dev fields, written under
     * voice_out_dev->lock, read in the Game URB callback).
     *
     * In piggyback mode the Game ISO callback advances pcm_pos_voice_out
     * at USB packet cadence, but ALSA's appl_ptr only moves when
     * userspace writes new frames.  PipeWire batches writes, so appl_ptr
     * can trail the consumed position; the pointer op then reports
     * hw_ptr > appl_ptr and snd_pcm_playback_avail() goes negative —
     * the underrun/stutter race seen in simultaneous Game + VO use.
     *
     * Fix: clamp the advance so hw_ptr never exceeds appl_ptr.  appl_ptr
     * is a 64-bit counter modulo runtime->boundary that does NOT restart
     * at 0 on prepare (snd_pcm_post_prepare sets appl_ptr = hw_ptr) and
     * can move backwards on RESET, while pcm_pos_voice_out is an absolute
     * 32-bit frame counter reset to 0 in driver prepare.  Direct
     * comparison is meaningless; instead keep an epoch base
     *   vo_appl_base = runtime->status->hw_ptr - pcm_pos_voice_out
     * (captured at piggyback engage and on prepare), so the frames the
     * callback may consume total
     *   budget = appl_ptr - vo_appl_base = queued + pcm_pos
     * where queued = appl_ptr - status->hw_ptr is userspace's committed
     * but unplayed prefetch.  The callback may advance pcm_pos_voice_out
     * only up to budget.  If userspace RESET moves appl_ptr backwards
     * (budget < 0), rebase to appl_ptr - pcm_pos (RESET implies
     * queued = 0); the callback tolerates the benign race and the next
     * callback re-clamps.
     */
    u64 vo_appl_base;   /* appl_ptr epoch origin for the VO piggyback clamp */

    /* Rate limiting for rapid open/close cycles from audio system probing */
    unsigned long last_open_jiffies;
    unsigned int open_count;

};

/* Global device pointers — defined in zg01_usb.c, accessed from zg01_pcm.c */
extern struct zg01_dev *game_dev;
extern struct zg01_dev *voice_in_dev;
extern struct zg01_dev *voice_out_dev;

/* Private workqueues — defined in zg01_usb.c */
extern struct workqueue_struct *zg01_cleanup_wq;
extern struct workqueue_struct *zg01_period_wq;

int zg01_create_pcm(struct zg01_dev *dev);
int zg01_set_streaming_interface(struct zg01_dev *dev, int interface, int alt_setting);
void zg01_stop_streaming(struct zg01_dev *dev);

/* Cleanup work functions — defined in zg01_pcm.c, registered in zg01_usb.c */
void zg01_cleanup_work_game_fn(struct work_struct *work);
void zg01_cleanup_work_voice_fn(struct work_struct *work);
void zg01_cleanup_work_voice_out_fn(struct work_struct *work);

/* Period-elapsed deferred work functions — defined in zg01_pcm.c, registered in zg01_usb.c */
void zg01_period_work_game_fn(struct work_struct *work);
void zg01_period_work_voice_fn(struct work_struct *work);
void zg01_period_work_voice_out_fn(struct work_struct *work);

/* USB Hardware Discovery Functions */
int zg01_discover_usb_config(struct zg01_dev *dev);
int zg01_find_audio_endpoint(struct zg01_dev *dev, u8 *endpoint_addr, u8 *alt_setting);

#endif /* ZG01_H */
