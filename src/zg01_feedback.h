/* SPDX-License-Identifier: GPL-2.0 */
#ifndef ZG01_FEEDBACK_H
#define ZG01_FEEDBACK_H

/* Pure protocol helpers, also compiled by the userspace sanitizer tests.
 * No payload is accessed before status and all bounds have been checked. */
static inline unsigned int
zg01_packet_frames(const unsigned char *buffer, unsigned int buffer_bytes,
                   unsigned int offset, unsigned int actual,
                   unsigned int requested, int status)
{
    const unsigned char *p;
    unsigned int payload;

    if (status || !buffer || actual < 92 || actual > 124 ||
        actual > requested || offset > buffer_bytes ||
        actual > buffer_bytes - offset)
        return 0;
    p = buffer + offset;
    payload = (unsigned int)p[4] | ((unsigned int)p[5] << 8) |
              ((unsigned int)p[6] << 16) | ((unsigned int)p[7] << 24);
    if (payload != actual - 12 || payload % 16)
        return 0;
    return payload / 16;
}

#define ZG01_FB_PACKETS 32
#define ZG01_FB_DEPTH 16

struct zg01_feedback_plan {
    unsigned char frames[ZG01_FB_PACKETS];
};

/* Caller serializes all operations, including reset, with dev->lock. */
struct zg01_feedback_queue {
    struct zg01_feedback_plan plan[ZG01_FB_DEPTH];
    unsigned int ids[ZG01_FB_DEPTH];
    unsigned int plan_head, plans, pending_head, pending;
};

static inline void zg01_feedback_reset(struct zg01_feedback_queue *q)
{
    q->plan_head = q->plans = q->pending_head = q->pending = 0;
}

static inline int zg01_feedback_push(struct zg01_feedback_queue *q,
                                     const struct zg01_feedback_plan *plan)
{
    if (q->plans == ZG01_FB_DEPTH)
        return 0;
    q->plan[(q->plan_head + q->plans++) % ZG01_FB_DEPTH] = *plan;
    return 1;
}

static inline int zg01_feedback_pending(struct zg01_feedback_queue *q,
                                        unsigned int id)
{
    if (q->pending == ZG01_FB_DEPTH)
        return 0;
    q->ids[(q->pending_head + q->pending++) % ZG01_FB_DEPTH] = id;
    return 1;
}

static inline int zg01_feedback_take(struct zg01_feedback_queue *q,
                                     struct zg01_feedback_plan *plan,
                                     unsigned int *id)
{
    if (!q->plans || !q->pending)
        return 0;
    *plan = q->plan[q->plan_head];
    *id = q->ids[q->pending_head];
    q->plan_head = (q->plan_head + 1) % ZG01_FB_DEPTH;
    q->pending_head = (q->pending_head + 1) % ZG01_FB_DEPTH;
    q->plans--;
    q->pending--;
    return 1;
}

/* Pop only a pending URB id, leaving queued plans untouched (gap fallback). */
static inline int zg01_feedback_pending_take(struct zg01_feedback_queue *q,
                                             unsigned int *id)
{
    if (!q->pending)
        return 0;
    *id = q->ids[q->pending_head];
    q->pending_head = (q->pending_head + 1) % ZG01_FB_DEPTH;
    q->pending--;
    return 1;
}

/* ALSA pointers share its boundary epoch; never compare appl_ptr to a
 * driver-local counter reset by prepare. A rewind/invalid distance is zero. */
static inline unsigned long
zg01_playback_available(unsigned long appl, unsigned long queued,
                        unsigned long boundary, unsigned long buffer_size)
{
    unsigned long distance;

    if (!boundary || appl >= boundary || queued >= boundary)
        return 0;
    distance = appl >= queued ? appl - queued : boundary - queued + appl;
    return distance <= buffer_size ? distance : 0;
}

#endif
