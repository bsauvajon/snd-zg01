/*
 * Yamaha ZG01 USB Audio Driver - meter reader
 *
 * The device sends 192-byte meter frames on EP 0x83 (interface 4) only
 * while the host keeps the 0x03 keepalive coming.  This file owns that
 * endpoint with a single URB that is resubmitted from its completion,
 * and a delayed work that sends the keepalive once a second.  Each valid
 * frame updates the four level snapshots read by the meter kcontrols.
 */

#include <linux/jiffies.h>
#include <linux/printk.h>
#include <linux/slab.h>
#include <linux/usb.h>

#include "zg01.h"
#include "zg01_meters.h"

#define ZG01_METER_MAX_ERRORS  16
#define ZG01_METER_STALE_MS    500

static void zg01_meters_store(struct zg01_meter_state *m,
                              const struct zg01_meter_values *v)
{
    atomic_set(&m->levels[ZG01_METER_MIC_IN], v->mic_in);
    atomic_set(&m->levels[ZG01_METER_MIC_OUT], v->mic_out);
    atomic_set(&m->levels[ZG01_METER_PC_IN], v->pc_in);
    atomic_set(&m->levels[ZG01_METER_STREAM_MIX], v->stream_mix);
    atomic_long_set(&m->valid_jiffies, jiffies);
    atomic_set(&m->valid, 1);
}

static void zg01_meters_complete(struct urb *urb)
{
    struct zg01_dev *dev = urb->context;
    struct zg01_meter_state *m = &dev->meters;
    struct zg01_meter_values v;
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
                print_hex_dump_debug("zg01 meter: ", DUMP_PREFIX_OFFSET,
                                     16, 1, urb->transfer_buffer,
                                     urb->actual_length, false);
            if (zg01_meter_parse(urb->transfer_buffer, urb->actual_length,
                                 &v) == 0)
                zg01_meters_store(m, &v);
            else
                m->invalid++;
        } else {
            m->other++;
        }

        if (m->frames == 1 ||
            time_after(jiffies, m->last_log_jiffies + HZ)) {
            m->last_log_jiffies = jiffies;
            dev_dbg(&dev->interface->dev,
                    "meter frames: %llu, invalid: %llu, other: %llu\n",
                    m->frames, m->invalid, m->other);
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

void zg01_meters_keepalive_fn(struct work_struct *work)
{
    struct zg01_dev *dev = container_of(work, struct zg01_dev,
                                        meters.keepalive_work.work);
    struct zg01_meter_state *m = &dev->meters;
    int ret;

    if (!m->keepalive_running || atomic_read(&dev->disconnecting))
        return;

    ret = zg01_param_keepalive(dev);
    if (ret)
        dev_warn_ratelimited(&dev->interface->dev,
                             "meter keepalive failed: %d\n", ret);

    mutex_lock(&dev->param_mutex);
    if (m->keepalive_running && !atomic_read(&dev->disconnecting))
        schedule_delayed_work(&m->keepalive_work, HZ);
    mutex_unlock(&dev->param_mutex);
}

int zg01_meters_start(struct zg01_dev *dev)
{
    struct zg01_meter_state *m = &dev->meters;
    int err;

    if (m->running)
        return 0;

    /* Release a URB left by an error stop before allocating another. */
    zg01_meters_stop(dev);

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
    m->invalid = 0;
    m->other = 0;
    m->last_log_jiffies = 0;
    atomic_set(&m->valid, 0);
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

    mutex_lock(&dev->param_mutex);
    m->keepalive_running = true;
    schedule_delayed_work(&m->keepalive_work, HZ);
    mutex_unlock(&dev->param_mutex);

    return 0;
}

void zg01_meters_stop(struct zg01_dev *dev)
{
    struct zg01_meter_state *m = &dev->meters;

    mutex_lock(&dev->param_mutex);
    m->keepalive_running = false;
    mutex_unlock(&dev->param_mutex);
    cancel_delayed_work_sync(&m->keepalive_work);

    m->running = false;
    if (m->urb) {
        usb_kill_urb(m->urb);
        usb_free_urb(m->urb);
        m->urb = NULL;
    }
    kfree(m->buf);
    m->buf = NULL;
    atomic_set(&m->valid, 0);
}

/*
 * Returns the last valid level for one meter (0..127), or -ENODATA when
 * no valid frame has arrived since start, when the reader is stopped, or
 * when the last valid frame is older than ZG01_METER_STALE_MS.  A missing
 * meter is never reported as zero.
 */
int zg01_meters_value(struct zg01_dev *dev, unsigned int index)
{
    struct zg01_meter_state *m = &dev->meters;

    if (index >= ZG01_METER_COUNT)
        return -EINVAL;
    if (!atomic_read(&m->valid))
        return -ENODATA;
    if (time_after(jiffies, atomic_long_read(&m->valid_jiffies) +
                   msecs_to_jiffies(ZG01_METER_STALE_MS)))
        return -ENODATA;

    return atomic_read(&m->levels[index]);
}
