/*
 * Yamaha ZG01 USB Audio Driver - USB glue
 *
 * One USB device, one ALSA card, three PCM devices (Game playback,
 * Voice Out playback, Voice In capture).  Probe anchors on interface 1
 * (shared EP 0x01 playback).  Interface 2 (EP 0x81 capture) is claimed
 * by the same driver instance and creates no extra card.  No global
 * device pointers, no devices_mutex.
 */

#include <linux/module.h>
#include <linux/slab.h>
#include <linux/string.h>
#include <linux/workqueue.h>
#include <sound/core.h>
#include <sound/pcm.h>

#include "zg01.h"

struct workqueue_struct *zg01_cleanup_wq;
struct workqueue_struct *zg01_period_wq;

static void zg01_card_private_free(struct snd_card *card)
{
    struct zg01_dev *dev = card->private_data;

    if (dev->udev) {
        usb_put_dev(dev->udev);
        dev->udev = NULL;
    }
}

static int zg01_probe(struct usb_interface *interface,
                      const struct usb_device_id *id)
{
    struct usb_device *udev = interface_to_usbdev(interface);
    struct snd_card *card;
    struct zg01_dev *dev;
    int err;

    if (interface->cur_altsetting->desc.bInterfaceNumber != 1)
        return 0;

    err = snd_card_new(&interface->dev, -1, "zg01", THIS_MODULE,
                       sizeof(struct zg01_dev), &card);
    if (err)
        return err;

    dev = card->private_data;
    dev->card = card;
    dev->udev = usb_get_dev(udev);
    dev->interface = interface;
    card->private_free = zg01_card_private_free;

    spin_lock_init(&dev->lock);
    mutex_init(&dev->state_mutex);
    atomic_set(&dev->disconnecting, 0);
    atomic_set(&dev->disconnected, 0);
    dev->device_initialized = false;
    dev->current_rate = 48000;
    dev->last_open_jiffies = 0;
    dev->open_count = 0;

    for (int i = 0; i < ZG01_N_STREAMS; i++) {
        struct zg01_stream *s = &dev->streams[i];

        s->dev = dev;
        s->substream = NULL;
        s->pcm_pos = 0;
        s->opened = false;
        s->initialized = false;
        s->running = false;
        s->last_trigger_jiffies = 0;
        s->trigger_count = 0;
        INIT_WORK(&s->period_work, zg01_period_work_fn);
        INIT_WORK(&s->xrun_work, zg01_xrun_work_fn);
    }
    dev->streams[ZG01_GAME].pcm_device = ZG01_PCM_GAME;
    dev->streams[ZG01_GAME].direction = SNDRV_PCM_STREAM_PLAYBACK;
    dev->streams[ZG01_VOICE_OUT].pcm_device = ZG01_PCM_VOICE_OUT;
    dev->streams[ZG01_VOICE_OUT].direction = SNDRV_PCM_STREAM_PLAYBACK;
    dev->streams[ZG01_VOICE_IN].pcm_device = ZG01_PCM_VOICE_IN;
    dev->streams[ZG01_VOICE_IN].direction = SNDRV_PCM_STREAM_CAPTURE;

    dev->out_chain.dev = dev;
    dev->in_chain.dev = dev;
    for (int i = 0; i < MAX_URBS; i++) {
        dev->out_chain.urbs[i] = NULL;
        dev->out_chain.bufs[i] = NULL;
        dev->in_chain.urbs[i] = NULL;
        dev->in_chain.bufs[i] = NULL;
    }
    dev->out_chain.allocated = false;
    dev->in_chain.allocated = false;
    dev->out_chain.state = ZG01_CHAIN_STOPPED;
    dev->in_chain.state = ZG01_CHAIN_STOPPED;
    atomic_set(&dev->out_chain.inflight, 0);
    atomic_set(&dev->in_chain.inflight, 0);
    INIT_WORK(&dev->out_chain.cleanup_work, zg01_chain_cleanup_fn);
    INIT_WORK(&dev->in_chain.cleanup_work, zg01_chain_cleanup_fn);
    INIT_DELAYED_WORK(&dev->out_chain.quiesce_work, zg01_chain_quiesce_fn);
    INIT_DELAYED_WORK(&dev->in_chain.quiesce_work, zg01_chain_quiesce_fn);

    strscpy(card->driver, "zg01_usb", sizeof(card->driver));
    strscpy(card->shortname, "ZG01", sizeof(card->shortname));
    strscpy(card->longname, "Yamaha ZG01", sizeof(card->longname));
    strscpy(card->mixername, "ZG01", sizeof(card->mixername));

    err = zg01_init_control(dev);
    if (err) {
        dev_err(&interface->dev, "control init failed: %d\n", err);
        goto err_free_card;
    }

    /* Discovery is best-effort, debug only */
    if (zg01_discover_usb_config(dev))
        dev_warn(&interface->dev, "discovery failed, continuing\n");

    /* Streaming interfaces to alt 0 before PCM registration */
    usb_set_interface(dev->udev, 1, 0);
    usb_set_interface(dev->udev, 2, 0);

    err = zg01_create_pcm_devices(dev);
    if (err) {
        dev_err(&interface->dev, "PCM creation failed: %d\n", err);
        goto err_free_card;
    }

    err = snd_card_register(card);
    if (err) {
        dev_err(&interface->dev, "card registration failed: %d\n", err);
        goto err_free_card;
    }

    usb_set_intfdata(interface, dev);
    dev_info(&interface->dev, "ZG01 card created (3 PCM devices)\n");
    return 0;

err_free_card:
    snd_card_free(card);
    return err;
}

static void zg01_disconnect(struct usb_interface *interface)
{
    struct zg01_dev *dev = usb_get_intfdata(interface);
    int i;

    if (!dev)
        return;

    usb_set_intfdata(interface, NULL);

    if (atomic_xchg(&dev->disconnected, 1))
        return;
    atomic_set(&dev->disconnecting, 1);

    /* Stop and drain everything that can call into ALSA or touch dev. */
    cancel_delayed_work_sync(&dev->out_chain.quiesce_work);
    cancel_delayed_work_sync(&dev->in_chain.quiesce_work);
    zg01_stop_all_chains(dev);
    zg01_drain_all_chains(dev);
    flush_workqueue(zg01_period_wq);
    for (i = 0; i < ZG01_N_STREAMS; i++) {
        flush_work(&dev->streams[i].period_work);
        flush_work(&dev->streams[i].xrun_work);
    }

    /* Mark chains unallocated under state_mutex so a
     * concurrent trigger/chain_start (blocked on the mutex) sees the
     * teardown and bails with -ENODEV instead of submitting URBs we
     * are about to free. */
    mutex_lock(&dev->state_mutex);
    dev->out_chain.allocated = false;
    dev->in_chain.allocated = false;
    mutex_unlock(&dev->state_mutex);

    /* Free URBs and buffers for real. */
    for (i = 0; i < MAX_URBS; i++) {
        if (dev->out_chain.urbs[i]) {
            usb_kill_urb(dev->out_chain.urbs[i]);
            usb_free_urb(dev->out_chain.urbs[i]);
            dev->out_chain.urbs[i] = NULL;
        }
        kfree(dev->out_chain.bufs[i]);
        dev->out_chain.bufs[i] = NULL;
        if (dev->in_chain.urbs[i]) {
            usb_kill_urb(dev->in_chain.urbs[i]);
            usb_free_urb(dev->in_chain.urbs[i]);
            dev->in_chain.urbs[i] = NULL;
        }
        kfree(dev->in_chain.bufs[i]);
        dev->in_chain.bufs[i] = NULL;
    }

    /* Asynchronous teardown: does not hang the USB hub thread when
     * PipeWire still holds the device open. */
    snd_card_disconnect(dev->card);
    snd_card_free_when_closed(dev->card);
}

static int zg01_suspend(struct usb_interface *intf, pm_message_t message)
{
    struct zg01_dev *dev = usb_get_intfdata(intf);

    if (!dev)
        return 0;

    zg01_suspend_pcm(dev);
    zg01_stop_all_chains(dev);
    zg01_drain_all_chains(dev);
    return 0;
}

static int zg01_resume(struct usb_interface *intf)
{
    /* reset_resume handles the real reinit; plain resume is a no-op
     * beyond the stream-state reset. */
    struct zg01_dev *dev = usb_get_intfdata(intf);

    if (dev)
        zg01_pm_reset_streams(dev);
    return 0;
}

/*
 * The ZG01 firmware resets itself across suspend, so without
 * reset_resume the core runs disconnect+probe and userspace sees
 * duplicate nodes.  Keep the interfaces bound and the card objects.
 */
static int zg01_reset_resume(struct usb_interface *intf)
{
    struct zg01_dev *dev = usb_get_intfdata(intf);

    if (!dev)
        return 0;

    zg01_pm_reset_streams(dev);

    /* Device-global reinit: vendor handshake + interfaces to alt 0. */
    zg01_init_control(dev);
    usb_set_interface(dev->udev, 1, 0);
    usb_set_interface(dev->udev, 2, 0);
    return 0;
}

static const struct usb_device_id zg01_table[] = {
    { USB_DEVICE(VENDOR_ID_YAMAHA, PRODUCT_ID_ZG01) },
    { }
};
MODULE_DEVICE_TABLE(usb, zg01_table);

static struct usb_driver zg01_driver = {
    .name          = "zg01_usb",
    .id_table      = zg01_table,
    .probe         = zg01_probe,
    .disconnect    = zg01_disconnect,
    .suspend       = zg01_suspend,
    .resume        = zg01_resume,
    .reset_resume  = zg01_reset_resume,
};

static int __init zg01_init(void)
{
    int ret;

    zg01_cleanup_wq = alloc_workqueue("zg01-cleanup", WQ_MEM_RECLAIM, 0);
    if (!zg01_cleanup_wq)
        return -ENOMEM;

    zg01_period_wq = alloc_workqueue("zg01-period", WQ_MEM_RECLAIM, 0);
    if (!zg01_period_wq) {
        destroy_workqueue(zg01_cleanup_wq);
        zg01_cleanup_wq = NULL;
        return -ENOMEM;
    }

    ret = usb_register(&zg01_driver);
    if (ret) {
        destroy_workqueue(zg01_period_wq);
        destroy_workqueue(zg01_cleanup_wq);
        zg01_period_wq = NULL;
        zg01_cleanup_wq = NULL;
        return ret;
    }

    return 0;
}

static void __exit zg01_exit(void)
{
    usb_deregister(&zg01_driver);
    destroy_workqueue(zg01_period_wq);
    destroy_workqueue(zg01_cleanup_wq);
}

module_init(zg01_init);
module_exit(zg01_exit);

MODULE_AUTHOR("Yamaha ZG01 Driver Contributors");
MODULE_DESCRIPTION("Yamaha ZG01 USB Audio Driver");
MODULE_LICENSE("GPL");
