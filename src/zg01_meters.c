/*
 * Yamaha ZG01 USB Audio Driver - meter reader
 *
 * The device sends 192-byte meter frames on EP 0x83 (interface 4) only
 * while the host keeps the 0x03 keepalive coming.  This file owns that
 * endpoint with a single URB that is resubmitted from its completion,
 * and a delayed work that sends the keepalive once a second.  Each valid
 * frame updates the four level snapshots read by the meter kcontrols.
 */

#include <linux/errno.h>
#include <linux/jiffies.h>
#include <linux/printk.h>
#include <linux/slab.h>
#include <linux/spinlock.h>
#include <linux/string.h>
#include <linux/usb.h>
#include <linux/workqueue.h>

#include "zg01.h"
#include "zg01_meters.h"
#include "zg01_state.h"

#define ZG01_METER_MAX_ERRORS  16
#define ZG01_METER_STALE_MS    500
#define ZG01_STATE_TIMEOUT_MS  2000

static void zg01_meters_store(struct zg01_meter_state *m,
                              const struct zg01_meter_values *v)
{
    /*
     * Publish the snapshot with release ordering: a reader that observes
     * valid == 1 with acquire ordering sees the levels and the timestamp
     * written before it, even on a weakly ordered CPU (arm64).
     */
    atomic_set(&m->levels[ZG01_METER_MIC_IN], v->mic_in);
    atomic_set(&m->levels[ZG01_METER_MIC_OUT], v->mic_out);
    atomic_set(&m->levels[ZG01_METER_PC_IN], v->pc_in);
    atomic_set(&m->levels[ZG01_METER_STREAM_MIX], v->stream_mix);
    atomic_long_set(&m->valid_jiffies, jiffies);
    atomic_set_release(&m->valid, 1);
}

/* The type-20 response frames start with this USB-MIDI header. */
static bool zg01_meters_state_frame(const unsigned char *buf,
                                    unsigned int len)
{
    return len == ZG01_STATE_FRAME_SIZE &&
           buf[0] == 0x04 && buf[1] == 0xf0 && buf[2] == 0x43 &&
           buf[3] == 0x00;
}

static void zg01_state_timeout_fn(struct work_struct *work)
{
    struct zg01_meter_state *m =
        container_of(work, struct zg01_meter_state, state_timeout.work);
    struct zg01_dev *dev = container_of(m, struct zg01_dev, meters);
    unsigned long flags;

    spin_lock_irqsave(&m->state_lock, flags);
    if (m->state_armed || m->state_ready) {
        m->state_armed = false;
        m->state_ready = false;
        WRITE_ONCE(dev->state_status, -ETIMEDOUT);
        WRITE_ONCE(dev->state_known, 0);
        dev_warn_ratelimited(&dev->interface->dev,
                             "state read timeout: frames=%u\n",
                             m->state.count);
    }
    spin_unlock_irqrestore(&m->state_lock, flags);
}

static void zg01_state_work_fn(struct work_struct *work)
{
    struct zg01_meter_state *m =
        container_of(work, struct zg01_meter_state, state_work);
    struct zg01_dev *dev = container_of(m, struct zg01_dev, meters);
    struct zg01_state_values values;
    unsigned long flags;
    int ret;

    if (atomic_read(&dev->disconnecting)) {
        spin_lock_irqsave(&m->state_lock, flags);
        m->state_ready = false;
        spin_unlock_irqrestore(&m->state_lock, flags);
        return;
    }

    /*
     * The timeout may have declared the read failed while this work was
     * queued; do not publish a late success over that terminal error.
     */
    spin_lock_irqsave(&m->state_lock, flags);
    if (!m->state_ready) {
        spin_unlock_irqrestore(&m->state_lock, flags);
        return;
    }
    spin_unlock_irqrestore(&m->state_lock, flags);

    ret = zg01_state_parse(m->state.frames, ZG01_STATE_DUMP_SIZE,
                           &values);
    if (!ret) {
        zg01_control_apply_state(dev, &values);
        WRITE_ONCE(dev->state_known, values.known);
        WRITE_ONCE(dev->state_status, 1);
    } else {
        WRITE_ONCE(dev->state_known, 0);
        WRITE_ONCE(dev->state_status, ret);
    }

    cancel_delayed_work(&m->state_timeout);
    spin_lock_irqsave(&m->state_lock, flags);
    m->state_ready = false;
    spin_unlock_irqrestore(&m->state_lock, flags);
}

/*
 * Ask the device for a MIC state dump.  The type-20 request is also the
 * meter-session init, so this doubles as the startup read.  Only one read
 * may be in flight: the eight frames must not interleave with another
 * read.  Returns -EBUSY when one is pending, -ENODEV without a device or
 * state buffer, or the bulk-out error.
 */
int zg01_state_request(struct zg01_dev *dev)
{
    struct zg01_meter_state *m;
    unsigned long flags;
    int ret;

    if (!dev || !dev->udev)
        return -ENODEV;
    m = &dev->meters;
    if (atomic_read(&dev->disconnecting))
        return -ENODEV;
    if (!READ_ONCE(m->running))
        return -EIO;

    mutex_lock(&m->state_mutex);
    spin_lock_irqsave(&m->state_lock, flags);
    if (m->state_armed || m->state_ready) {
        spin_unlock_irqrestore(&m->state_lock, flags);
        mutex_unlock(&m->state_mutex);
        return -EBUSY;
    }
    spin_unlock_irqrestore(&m->state_lock, flags);

    /*
     * Admission is settled: cancel the previous read's timeout only now.
     * A rejected duplicate above must leave the active request's timeout
     * armed, otherwise the active read can never time out.
     */
    cancel_delayed_work_sync(&m->state_timeout);

    spin_lock_irqsave(&m->state_lock, flags);
    m->state.count = 0;
    m->state_armed = true;
    m->state_ready = false;
    WRITE_ONCE(dev->state_status, 0);
    WRITE_ONCE(dev->state_known, 0);
    spin_unlock_irqrestore(&m->state_lock, flags);

    schedule_delayed_work(&m->state_timeout,
                          msecs_to_jiffies(ZG01_STATE_TIMEOUT_MS));
    mutex_unlock(&m->state_mutex);

    ret = zg01_param_meter_init(dev);
    if (ret) {
        spin_lock_irqsave(&m->state_lock, flags);
        m->state_armed = false;
        spin_unlock_irqrestore(&m->state_lock, flags);
        cancel_delayed_work_sync(&m->state_timeout);
        WRITE_ONCE(dev->state_status, ret);
    }
    return ret;
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
            WRITE_ONCE(m->running, false);
            return;
        }
    } else {
        m->errors = 0;
        if (urb->actual_length == ZG01_STATE_FRAME_SIZE) {
            bool header = zg01_meters_state_frame(urb->transfer_buffer,
                                                  urb->actual_length);
            bool complete = false;
            unsigned long flags;

            spin_lock_irqsave(&m->state_lock, flags);
            /*
             * Only the first frame carries the type-20 header.  The
             * seven continuation frames are accepted while armed; a
             * 512-byte frame before the first header is not state.
             */
            if (m->state_armed &&
                m->state.count < ZG01_STATE_FRAME_COUNT &&
                (m->state.count > 0 || header)) {
                if (zg01_state_collect(&m->state, urb->transfer_buffer,
                                       urb->actual_length) == 0 &&
                    m->state.count == ZG01_STATE_FRAME_COUNT) {
                    m->state_armed = false;
                    m->state_ready = true;
                    complete = true;
                }
            }
            spin_unlock_irqrestore(&m->state_lock, flags);
            if (complete)
                schedule_work(&m->state_work);
        } else if (urb->actual_length == ZG01_METER_FRAME_LEN) {
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

    if (atomic_read(&dev->disconnecting) || !READ_ONCE(m->running))
        return;

    err = usb_submit_urb(urb, GFP_ATOMIC);
    if (err == -EPERM)
        return;
    if (err) {
        WRITE_ONCE(m->running, false);
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

    if (!READ_ONCE(m->keepalive_running) || atomic_read(&dev->disconnecting))
        return;

    ret = zg01_param_keepalive(dev);
    if (ret)
        dev_warn_ratelimited(&dev->interface->dev,
                             "meter keepalive failed: %d\n", ret);

    mutex_lock(&dev->param_mutex);
    if (READ_ONCE(m->keepalive_running) && !atomic_read(&dev->disconnecting))
        schedule_delayed_work(&m->keepalive_work, HZ);
    mutex_unlock(&dev->param_mutex);
}

/*
 * Periodic status poll on EP0.  The app sends vendor request 7 and then
 * request 4 about every 250 ms, from app start through the whole meter
 * session (Windows capture 00-mic-baseline); the driver sent request 7
 * only once at probe.  The transfer uses EP0, so it does not take
 * param_mutex; the mutex only guards the running flag and the re-arm.
 */
void zg01_meters_poll_fn(struct work_struct *work)
{
    struct zg01_dev *dev = container_of(work, struct zg01_dev,
                                        meters.poll_work.work);
    struct zg01_meter_state *m = &dev->meters;
    int ret;

    if (!READ_ONCE(m->poll_running) || atomic_read(&dev->disconnecting))
        return;

    ret = zg01_status_poll(dev);
    if (ret)
        dev_warn_ratelimited(&dev->interface->dev,
                             "meter status poll failed: %d\n", ret);

    mutex_lock(&dev->param_mutex);
    if (READ_ONCE(m->poll_running) && !atomic_read(&dev->disconnecting))
        schedule_delayed_work(&m->poll_work, HZ / 4);
    mutex_unlock(&dev->param_mutex);
}

int zg01_meters_start(struct zg01_dev *dev)
{
    struct zg01_meter_state *m = &dev->meters;
    int err;

    if (!dev || !dev->param_available)
        return 0;

    if (READ_ONCE(m->running))
        return 0;

    /*
     * Initialize the state collector before the stop below: the stop
     * cancels these works, and a restart must cancel the previous read.
     */
    spin_lock_init(&m->state_lock);
    mutex_init(&m->state_mutex);
    INIT_WORK(&m->state_work, zg01_state_work_fn);
    INIT_DELAYED_WORK(&m->state_timeout, zg01_state_timeout_fn);

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

    m->state.count = 0;
    m->state_armed = false;
    m->state_ready = false;
    WRITE_ONCE(dev->state_status, 0);
    WRITE_ONCE(dev->state_known, 0);

    usb_fill_bulk_urb(m->urb, dev->udev,
                      usb_rcvbulkpipe(dev->udev, ZG01_PARAM_EP_IN),
                      m->buf, ZG01_PARAM_FRAME,
                      zg01_meters_complete, dev);

    m->errors = 0;
    m->frames = 0;
    m->invalid = 0;
    m->other = 0;
    m->last_log_jiffies = 0;
    atomic_set_release(&m->valid, 0);
    WRITE_ONCE(m->running, true);

    err = usb_submit_urb(m->urb, GFP_KERNEL);
    if (err) {
        WRITE_ONCE(m->running, false);
        usb_free_urb(m->urb);
        m->urb = NULL;
        kfree(m->buf);
        m->buf = NULL;
        return err;
    }

    err = zg01_state_request(dev);
    if (err)
        dev_warn_ratelimited(&dev->interface->dev,
                             "state read start failed: %d\n", err);

    mutex_lock(&dev->param_mutex);
    WRITE_ONCE(m->poll_running, true);
    schedule_delayed_work(&m->poll_work, 0);
    WRITE_ONCE(m->keepalive_running, true);
    schedule_delayed_work(&m->keepalive_work, HZ);
    mutex_unlock(&dev->param_mutex);

    return 0;
}

void zg01_meters_stop(struct zg01_dev *dev)
{
    struct zg01_meter_state *m = &dev->meters;

    mutex_lock(&dev->param_mutex);
    WRITE_ONCE(m->keepalive_running, false);
    mutex_unlock(&dev->param_mutex);
    cancel_delayed_work_sync(&m->keepalive_work);

    mutex_lock(&dev->param_mutex);
    WRITE_ONCE(m->poll_running, false);
    mutex_unlock(&dev->param_mutex);
    cancel_delayed_work_sync(&m->poll_work);

    WRITE_ONCE(m->running, false);
    if (m->urb) {
        usb_kill_urb(m->urb);
        usb_free_urb(m->urb);
        m->urb = NULL;
    }

    /*
     * The URB is dead, so no new completion can queue the state work.
     * Cancel the timeout and the parser before resetting the collector.
     */
    cancel_delayed_work_sync(&m->state_timeout);
    cancel_work_sync(&m->state_work);
    {
        unsigned long flags;

        spin_lock_irqsave(&m->state_lock, flags);
        m->state_armed = false;
        m->state_ready = false;
        m->state.count = 0;
        spin_unlock_irqrestore(&m->state_lock, flags);
    }

    kfree(m->buf);
    m->buf = NULL;
    atomic_set_release(&m->valid, 0);
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
    if (!atomic_read_acquire(&m->valid))
        return -ENODATA;
    if (time_after(jiffies, atomic_long_read(&m->valid_jiffies) +
                   msecs_to_jiffies(ZG01_METER_STALE_MS)))
        return -ENODATA;

    return atomic_read(&m->levels[index]);
}
