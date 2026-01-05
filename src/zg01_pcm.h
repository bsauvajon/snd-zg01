#ifndef AUDIO_H
#define AUDIO_H

#include <sound/pcm.h>

#define USB_N_URBS 4
#define USB_N_PACKETS_PER_URB 16
#define USB_PACKET_SIZE 360
#define USB_BUFFER_SIZE (USB_PACKET_SIZE * USB_N_PACKETS_PER_URB)

#define BYTES_PER_PERIOD 3528
#define PERIODS_MAX 128
#define ALSA_BUFFER_SIZE (BYTES_PER_PERIOD * PERIODS_MAX)

struct zg01;

struct zg01_urb {
	struct zg01 *bcd2k;
	struct zg01_substream *stream;

	/* BEGIN DO NOT SEPARATE */
	struct urb instance;
	struct usb_iso_packet_descriptor packets[USB_N_PACKETS_PER_URB];
	/* END DO NOT SEPARATE */
	u8 *buffer;
};

struct zg01_substream {
	struct snd_pcm_substream *instance;

	u8 state;
	bool active;
	snd_pcm_uframes_t dma_off; /* current position in alsa dma_area */
	snd_pcm_uframes_t period_off; /* current position in current period */

	struct zg01_urb urbs[USB_N_URBS];

	spinlock_t lock;
	struct mutex mutex;
	wait_queue_head_t wait_queue;
	bool wait_cond;
};

struct zg01_pcm {
	struct zg01_dev *zg01;

	struct snd_pcm *instance;
	struct snd_pcm_hardware pcm_info;

	struct zg01_substream playback;
	struct zg01_substream capture;
	bool panic; /* if set driver won't do anymore pcm on device */
};

int zg01_init_audio(struct zg01 *zg01_dev);
void zg01_free_audio(struct zg01 *zg01_dev);

#endif