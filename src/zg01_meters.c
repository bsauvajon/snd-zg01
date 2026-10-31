/*
 * Yamaha ZG01 USB Audio Driver - meter reader
 *
 * The device sends 192-byte meter frames on EP 0x83 (interface 4) only
 * while the host keeps IN URBs queued there.  This file owns that
 * endpoint with a single URB that is resubmitted from its completion.
 *
 * Capture-only: frames are counted and logged, not decoded yet.
 */

#include <linux/jiffies.h>
#include <linux/printk.h>
#include <linux/slab.h>
#include <linux/usb.h>

#include "zg01.h"

#define ZG01_METER_FRAME_LEN   192
#define ZG01_METER_MAX_ERRORS  16

static void zg01_meters_complete(struct urb *urb)
{
    struct zg01_dev *dev = urb->context;
    struct zg01_meter_state *m = &dev->meters;
    int err;

    if (urb->status == -ESHUTDOWN || urb->status == -ENOENT)
        return;

    if (urb->status) {
        m->errors++;
        dev_warn_ratelimited(&dev->interface->dev,
                             "meter URB failed: %d\n", urb->status);
        if (m->errors >= ZG01_METER_MAX_ERRORS) {
            dev_err(&dev->interface->dev,
                    "meter reader stopped after %u errors\n", m->errors);
            m->running = false;
            return;
        }
    } else {
        m->errors = 0;
        if (urb->actual_length == ZG01_METER_FRAME_LEN) {
            m->frames++;
            if (m->frames == 1)
                print_hex_dump(KERN_INFO, "zg01 meter: ",
                               DUMP_PREFIX_OFFSET, 16, 1,
                               urb->transfer_buffer, urb->actual_length,
                               false);
        } else {
            m->other++;
        }

        if (m->frames == 1 ||
            time_after(jiffies, m->last_log_jiffies + HZ)) {
            m->last_log_jiffies = jiffies;
            dev_info(&dev->interface->dev,
                     "meter frames: %llu (192 bytes), other: %llu\n",
                     m->frames, m->other);
        }
    }

    if (atomic_read(&dev->disconnecting) || !m->running)
        return;

    err = usb_submit_urb(urb, GFP_ATOMIC);
    if (err == -EPERM)
        return;
    if (err) {
        m->running = false;
        dev_err(&dev->interface->dev,
                "meter URB resubmit failed: %d\n", err);
    }
}

int zg01_meters_start(struct zg01_dev *dev)
{
    struct zg01_meter_state *m = &dev->meters;
    int err;

    if (m->running)
        return 0;

    m->urb = usb_alloc_urb(0, GFP_KERNEL);
    if (!m->urb)
        return -ENOMEM;

    m->buf = kmalloc(ZG01_PARAM_FRAME, GFP_KERNEL);
    if (!m->buf) {
        usb_free_urb(m->urb);
        m->urb = NULL;
        return -ENOMEM;
    }

    usb_fill_bulk_urb(m->urb, dev->udev,
                      usb_rcvbulkpipe(dev->udev, ZG01_PARAM_EP_IN),
                      m->buf, ZG01_PARAM_FRAME,
                      zg01_meters_complete, dev);

    m->errors = 0;
    m->frames = 0;
    m->other = 0;
    m->last_log_jiffies = 0;
    m->running = true;

    err = usb_submit_urb(m->urb, GFP_KERNEL);
    if (err) {
        m->running = false;
        usb_free_urb(m->urb);
        m->urb = NULL;
        kfree(m->buf);
        m->buf = NULL;
        return err;
    }

    return 0;
}

void zg01_meters_stop(struct zg01_dev *dev)
{
    struct zg01_meter_state *m = &dev->meters;

    m->running = false;
    if (m->urb) {
        usb_kill_urb(m->urb);
        usb_free_urb(m->urb);
        m->urb = NULL;
    }
    kfree(m->buf);
    m->buf = NULL;
}
