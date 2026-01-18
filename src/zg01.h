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
    int card_index;

    struct zg01_midi *midi;
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
    int active_urbs_game;
    
    /* Voice channel (low bandwidth) - multiple URBs for stability */  
    struct urb *iso_urbs_voice[MAX_URBS_PER_CHANNEL];
    unsigned char *iso_buffers_voice[MAX_URBS_PER_CHANNEL];
    dma_addr_t iso_dmas_voice[MAX_URBS_PER_CHANNEL];
    int active_urbs_voice;
    
    /* Voice output channel (playback to voice output) - multiple URBs for stability */
    struct urb *iso_urbs_voice_out[MAX_URBS_PER_CHANNEL];
    unsigned char *iso_buffers_voice_out[MAX_URBS_PER_CHANNEL];
    dma_addr_t iso_dmas_voice_out[MAX_URBS_PER_CHANNEL];
    int active_urbs_voice_out;
    
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
    unsigned long game_startup_frames; /* Count frames during startup to allow buffer fill */
    unsigned long voice_startup_frames;
    unsigned long voice_out_startup_frames;
    
    unsigned int current_rate;      /* Current sample rate (44100 or 48000) */
    unsigned int rate_residual;     /* Fractional sample accumulator */
    
    bool cleanup_in_progress_game;
    bool cleanup_in_progress_voice;
    bool cleanup_in_progress_voice_out;
    unsigned long last_trigger_jiffies;
    
    /* Trigger loop detection - per-device to avoid race conditions */
    unsigned long last_trigger_time;
    int trigger_count;
    
    /* Rate limiting for rapid open/close cycles from audio system probing */
    unsigned long last_open_jiffies;
    unsigned int open_count;

    /* Workqueue for deferred URB cleanup to avoid sleeping in atomic contexts */
    struct workqueue_struct *wq;

    /* Deferred start support to debounce user-space probing */
    struct delayed_work start_work_game;
    struct delayed_work start_work_voice;
    struct delayed_work start_work_voice_out;
    bool start_pending_game;
    bool start_pending_voice;
    bool start_pending_voice_out;
};

int zg01_create_pcm(struct zg01_dev *dev);
int zg01_set_streaming_interface(struct zg01_dev *dev, int interface, int alt_setting);

/* USB Hardware Discovery Functions */
int zg01_discover_usb_config(struct zg01_dev *dev);
int zg01_find_audio_endpoint(struct zg01_dev *dev, u8 *endpoint_addr, u8 *alt_setting);

#endif /* ZG01_H */
