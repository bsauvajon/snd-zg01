#include <linux/module.h>
#include <linux/usb.h>
#include <linux/slab.h>
#include <sound/core.h>
#include <sound/pcm.h>
#include <sound/pcm_params.h>
#include <linux/bitmap.h>
#include "zg01.h" // Include your header for shared structs and defs
#include "zg01_pcm.h"
#include "zg01_control.h"

#define VENDOR_ID_YAMAHA 0x0499
#define PRODUCT_ID_ZG01 0x1513

static DEFINE_MUTEX(devices_mutex);
DECLARE_BITMAP(devices_used, SNDRV_CARDS);

/* Track which cards have been created for this device */
static struct zg01_dev *game_dev = NULL;
static struct zg01_dev *voice_in_dev = NULL;
static struct zg01_dev *voice_out_dev = NULL;

static int zg01_probe(struct usb_interface *interface,
                      const struct usb_device_id *id)
{
    struct zg01_dev *dev;
    struct snd_card *card;
    int err;
    unsigned int card_index;
    int iface_num;
    int channel_type; /* 0=game, 1=voice_in, 2=voice_out */

    /* Create sound cards for Game (Interface 1), Voice In (Interface 2), and Voice Out (Interface 1 alt config) */
    iface_num = interface->cur_altsetting->desc.bInterfaceNumber;
    /* Lock to protect global device pointer access */
    mutex_lock(&devices_mutex);
    if (iface_num != 1 && iface_num != 2) {
        dev_info(&interface->dev, "ZG01: Skipping interface %d (not Game/Voice)\n", iface_num);
        mutex_unlock(&devices_mutex); return 0; /* Success but no card created */
    }

    /* Interface 1 creates TWO cards: Game (playback) and Voice Out (playback)
     * Interface 2 creates ONE card: Voice In (capture) */
    if (iface_num == 1) {
        /* Create Game playback card first */
        if (!game_dev) {
            channel_type = CHANNEL_TYPE_GAME;
            dev_info(&interface->dev, "Yamaha ZG01 Game channel detected (interface %d)\n", iface_num);
        } else if (!voice_out_dev) {
            /* Create Voice Out playback card second */
            channel_type = CHANNEL_TYPE_VOICE_OUT;
            dev_info(&interface->dev, "Yamaha ZG01 Voice Out channel detected (interface %d)\n", iface_num);
        } else {
            /* Both cards already created for interface 1 */
            mutex_unlock(&devices_mutex); return 0;
        }
    } else {
        /* Interface 2 - Voice In capture */
        if (voice_in_dev) {
            mutex_unlock(&devices_mutex); return 0; /* Already created */
        }
        channel_type = CHANNEL_TYPE_VOICE_IN;
        dev_info(&interface->dev, "Yamaha ZG01 Voice In channel detected (interface %d)\n", iface_num);
    }

    for (card_index = 0; card_index < SNDRV_CARDS; ++card_index)
        if (!test_bit(card_index, devices_used))
            break;
    
    /* Create distinctive card ID based on channel type */
    const char *card_id;
    if (channel_type == CHANNEL_TYPE_GAME) {
        card_id = "zg01game";
    } else if (channel_type == CHANNEL_TYPE_VOICE_IN) {
        card_id = "zg01voice";  
    } else {
        card_id = "zg01voiceout";
    }

    /* Create card with embedded zg01_dev structure */
    err = snd_card_new(&interface->dev, -1, card_id, THIS_MODULE,
                       sizeof(struct zg01_dev), &card);
    if (err) {
        dev_err(&interface->dev, "Failed to create sound card: %d\n", err);
        return err;
    }

    /* Use the dev structure embedded in the card - this is critical! */
    dev = card->private_data;
    dev->card = card;
    dev->card_index = card_index;
    dev->channel_type = channel_type;
    
    /* Initialize dev structure */
    dev->udev = usb_get_dev(interface_to_usbdev(interface));
    dev->interface = interface;
    spin_lock_init(&dev->lock);
    mutex_init(&dev->pcm_mutex);
    dev->game_channel_active = false;
    dev->voice_channel_active = false;
    dev->voice_out_channel_active = false;
    dev->game_initialized = false;
    dev->voice_initialized = false;
    dev->voice_out_initialized = false;
    dev->cleanup_in_progress_game = false;
    dev->cleanup_in_progress_voice = false;
    dev->cleanup_in_progress_voice_out = false;
    INIT_DELAYED_WORK(&dev->start_work_game, (void *)0);
    INIT_DELAYED_WORK(&dev->start_work_voice, (void *)0);
    INIT_DELAYED_WORK(&dev->start_work_voice_out, (void *)0);
    dev->start_pending_game = false;
    dev->start_pending_voice = false;
    dev->start_pending_voice_out = false;

    /* Track device pointers globally */
    if (channel_type == CHANNEL_TYPE_GAME) {
        game_dev = dev;
    } else if (channel_type == CHANNEL_TYPE_VOICE_IN) {
        voice_in_dev = dev;
    } else {
        voice_out_dev = dev;
    }

    /* Unlock mutex - critical section complete */
    mutex_unlock(&devices_mutex);
    usb_set_intfdata(interface, dev);

    snd_card_set_dev(card, &interface->dev);

    strncpy(card->driver, "zg01_usb", sizeof(card->driver));
    
    /* Set distinctive card names based on channel type immediately */
    if (channel_type == CHANNEL_TYPE_GAME) {
        strncpy(card->shortname, "ZG01 Game", sizeof(card->shortname));
        strncpy(card->longname, "Yamaha ZG01 Game Channel", sizeof(card->longname));
        strncpy(card->mixername, "ZG01 Game", sizeof(card->mixername));
        strncpy(card->components, "USB0499:1513-Game", sizeof(card->components));
    } else if (channel_type == CHANNEL_TYPE_VOICE_IN) {
        strncpy(card->shortname, "ZG01 Voice In", sizeof(card->shortname));
        strncpy(card->longname, "Yamaha ZG01 Voice Input Channel", sizeof(card->longname));
        strncpy(card->mixername, "ZG01 Voice In", sizeof(card->mixername));
        strncpy(card->components, "USB0499:1513-VoiceIn", sizeof(card->components));
    } else {
        strncpy(card->shortname, "ZG01 Voice Out", sizeof(card->shortname));
        strncpy(card->longname, "Yamaha ZG01 Voice Output Channel", sizeof(card->longname));
        strncpy(card->mixername, "ZG01 Voice Out", sizeof(card->mixername));
        strncpy(card->components, "USB0499:1513-VoiceOut", sizeof(card->components));
    }

    err = zg01_init_control(dev);
    if (err) {
        dev_err(&interface->dev, "Failed to initialize control interface: %d\n", err);
        snd_card_free(card);
        return err;
    }    

    /* Discover USB hardware configuration */
    err = zg01_discover_usb_config(dev);
    if (err) {
        pr_warn("zg01_usb: USB discovery failed, continuing anyway: %d\n", err);
    }

    /* Set interface alternate settings for audio streaming */
    err = usb_set_interface(dev->udev, 1, 0);
    if (err) {
        dev_err(&interface->dev, "Failed to set interface 1 alt 0: %d\n", err);
    }
    
    err = usb_set_interface(dev->udev, 2, 0);  
    if (err) {
        dev_err(&interface->dev, "Failed to set interface 2 alt 0: %d\n", err);
    }

    err = zg01_create_pcm(dev);
    if (err) {
        dev_err(&interface->dev, "Failed to create PCM device: %d\n", err);
        snd_card_free(card);
        return err;
    }

    err = snd_card_register(card);
    if (err < 0) {
        dev_err(&interface->dev, "Failed to register sound card: %d\n", err);
        snd_card_free(card);  /* This frees the embedded dev structure */
        return err;
    }

    /* For interface 1, probe again to create the voice output card */
    if (iface_num == 1 && channel_type == CHANNEL_TYPE_GAME) {
        dev_info(&interface->dev, "ZG01: Probing interface 1 again for voice output\n");
        return zg01_probe(interface, id);
    }

    return 0;
}

static void zg01_disconnect(struct usb_interface *interface)
{
    struct zg01_dev *dev = usb_get_intfdata(interface);
    int iface_num = interface->cur_altsetting->desc.bInterfaceNumber;

    usb_set_intfdata(interface, NULL);

    if (!dev)
        return;

    /* Clean up all channels for this interface */
    int i;
    
    /* Clean up Game channel URBs */
    if (dev->channel_type == CHANNEL_TYPE_GAME || iface_num == 1) {
        for (i = 0; i < MAX_URBS_PER_CHANNEL; i++) {
            if (dev->iso_urbs_game[i]) {
                usb_kill_urb(dev->iso_urbs_game[i]);
                usb_free_urb(dev->iso_urbs_game[i]);
                dev->iso_urbs_game[i] = NULL;
            }
            if (dev->iso_buffers_game[i]) {
                dev->iso_buffers_game[i] = NULL;
            }
        }
    }
    
    /* Clean up Voice In channel URBs */
    if (dev->channel_type == CHANNEL_TYPE_VOICE_IN || iface_num == 2) {
        for (i = 0; i < MAX_URBS_PER_CHANNEL; i++) {
            if (dev->iso_urbs_voice[i]) {
                usb_kill_urb(dev->iso_urbs_voice[i]);
                usb_free_urb(dev->iso_urbs_voice[i]);
                dev->iso_urbs_voice[i] = NULL;
            }
            if (dev->iso_buffers_voice[i]) {
                dev->iso_buffers_voice[i] = NULL;
            }
        }
    }
    
    /* Clean up Voice Out channel URBs */
    if (dev->channel_type == CHANNEL_TYPE_VOICE_OUT || iface_num == 1) {
        for (i = 0; i < MAX_URBS_PER_CHANNEL; i++) {
            if (dev->iso_urbs_voice_out[i]) {
                usb_kill_urb(dev->iso_urbs_voice_out[i]);
                usb_free_urb(dev->iso_urbs_voice_out[i]);
                dev->iso_urbs_voice_out[i] = NULL;
            }
            if (dev->iso_buffers_voice_out[i]) {
                dev->iso_buffers_voice_out[i] = NULL;
            }
        }
    }

    /* Clear global device pointers */
    if (dev == game_dev) {
        game_dev = NULL;
    } else if (dev == voice_in_dev) {
        voice_in_dev = NULL;
    } else if (dev == voice_out_dev) {
        voice_out_dev = NULL;
    }

    /* Free the card - this will also free the embedded dev structure */
    if (dev->card) {
        snd_card_free(dev->card);
    }

    dev_info(&interface->dev, "Yamaha ZG01 device disconnected\n");
}

int zg01_set_streaming_interface(struct zg01_dev *dev, int interface, int alt_setting)
{
    int ret;
    
    if (!dev || !dev->udev) {
        return -ENODEV;
    }
    
    ret = usb_set_interface(dev->udev, interface, alt_setting);
    if (ret) {
        dev_err(&dev->udev->dev, "Failed to set interface %d alt %d: %d\n", 
                interface, alt_setting, ret);
        return ret;
    }
    
    dev_dbg(&dev->udev->dev, "Set interface %d to alternate setting %d\n", 
            interface, alt_setting);
    return 0;
}

static struct usb_device_id zg01_table[] = {
    {USB_DEVICE(VENDOR_ID_YAMAHA, PRODUCT_ID_ZG01)},
    {}};
MODULE_DEVICE_TABLE(usb, zg01_table);

static struct usb_driver zg01_driver = {
    .name = "zg01_usb",
    .id_table = zg01_table,
    .probe = zg01_probe,
    .disconnect = zg01_disconnect,
};

module_usb_driver(zg01_driver);

MODULE_AUTHOR("Your Name");
MODULE_DESCRIPTION("Yamaha ZG01 USB Audio Driver");
MODULE_LICENSE("GPL");
