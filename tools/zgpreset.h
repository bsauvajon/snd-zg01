#ifndef ZGPRESET_H
#define ZGPRESET_H

#include <stddef.h>
#include <stdint.h>

/*
 * One addressable device parameter from a .zgpreset file.
 * The parser reports raw device values; it does not map addresses to names
 * or convert values to user units, and it does not read .zg01 backups.
 */
struct zgpreset_param {
    uint16_t address;
    uint8_t width;
    uint32_t value;
};

struct zgpreset {
    char block[32];
    /* Display name, allocated from the General payload at its full length
     * so a valid name is never truncated (including a final multi-byte
     * UTF-8 sequence).  NULL when the file carries none. */
    char *name;
    struct zgpreset_param *params;
    size_t count;
};

/*
 * Parse a Yamaha .zgpreset file.
 *
 * Returns 0 on success, -EINVAL for malformed input, -ENOTSUP for a .zg01
 * backup (multi-group F_CurrentScene is a follow-up), -ENOMEM on allocation
 * failure and -errno for I/O errors.  On failure *out is set to NULL.
 */
int zgpreset_open(const char *path, struct zgpreset **out);

/* Preset metadata; an empty string when the file carries none. */
const char *zgpreset_block(const struct zgpreset *preset);
const char *zgpreset_name(const struct zgpreset *preset);

void zgpreset_close(struct zgpreset *preset);

#endif /* ZGPRESET_H */
