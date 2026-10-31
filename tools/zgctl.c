/*
 * zgctl - command line control for the Yamaha ZG01 driver.
 *
 * Talks to the ALSA control device directly through its ioctls, so it
 * needs no libasound at build time.  It lists the driver's mixer
 * controls and reads/writes them with human units: dB for EQ band
 * gains, Hz for frequencies, Q for bandwidths.
 *
 * Build:  make -C tools
 * Usage:  zgctl list
 *         zgctl get "EQ Band 1 Gain"
 *         zgctl set "EQ Band 1 Gain" +6.0
 *         zgctl set "EQ Band 2 Frequency" 1k
 *         zgctl set "EQ Band 3 Q" 2.0
 *         zgctl set "Limiter Capture Switch" on
 *         zgctl set "Gate Threshold" 40     (device steps, see
 *         zgctl set "Compressor Ratio" 30    docs/MIC_CONTROL_PROTOCOL.md)
 *         zgctl preset list FILE
 *         zgctl preset show FILE
 *         zgctl preset load FILE [--dry-run] [--block NAME] [--allow-raw]
 */

#include <ctype.h>
#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>
#include <sys/ioctl.h>
#include <unistd.h>

#include <sound/asound.h>

#include "zgpreset.h"
#include "../src/zg01_codec.h"

enum kind {
    KIND_INT,
    KIND_BOOL,
    KIND_GAIN,   /* value is 0.1 dB */
    KIND_FREQ,   /* value is Hz */
    KIND_Q,      /* value is 0.01 Q */
};

/*
 * The generated control map is data only: it assumes these definitions.
 * The ALSA element type is not part of the map; switches are the only
 * controls whose type matters here.
 */
#define ZG01_CTL_RAW      (1u << 0)
#define ZG01_CTL_READONLY (1u << 1)

struct zg01_user_control {
    uint16_t address;
    const char *name;
    long min;
    long max;
    int conv;
    unsigned int def;
    unsigned int flags;
};

#include "zg01_controls_generated.h"

static const char *progname = "zgctl";

static void usage(void)
{
    fprintf(stderr,
            "usage: %s [-c CARD] list\n"
            "       %s [-c CARD] get CONTROL\n"
            "       %s [-c CARD] set CONTROL VALUE\n"
            "       %s [-c CARD] save\n"
            "       %s [-c CARD] reset\n"
            "       %s preset list FILE\n"
            "       %s preset show FILE\n"
            "       %s preset load FILE [--dry-run] [--block NAME] [--allow-raw]\n"
            "\n"
            "CARD defaults to the card whose driver is zg01_usb.\n"
            "preset list/show need no card; load opens it after planning.\n"
            "load writes each applicable control and prints applied/skipped/failed;\n"
            "it does not save the settings, use 'save' for that.\n"
            "VALUES: Gain in dB (+6.0), Frequency in Hz (1k, 7.5kHz),\n"
            "        Q as a ratio (2.0), switches on/off.\n"
            "save persists the current settings to the device;\n"
            "reset reloads the persisted settings.\n",
            progname, progname, progname, progname, progname, progname,
            progname, progname);
}

/* Find a card by id or driver name in /proc/asound/cards. */
static int find_card(const char *want)
{
    FILE *f = fopen("/proc/asound/cards", "r");
    char line[256];
    int idx;
    char id[64], drv[64];

    if (!f)
        return -1;

    while (fgets(line, sizeof(line), f)) {
        if (sscanf(line, " %d [%63[^]] ]: %63s", &idx, id, drv) != 3)
            continue;

        /* trim trailing spaces in the id field */
        for (char *p = id + strlen(id); p > id && p[-1] == ' '; p--)
            p[-1] = '\0';

        if (want) {
            if (!strcmp(id, want) || !strcmp(drv, want)) {
                fclose(f);
                return idx;
            }
        } else if (strstr(drv, "zg01")) {
            fclose(f);
            return idx;
        }
    }

    fclose(f);
    return -1;
}

static int ends_with(const char *s, const char *suffix)
{
    size_t a = strlen(s), b = strlen(suffix);

    return a >= b && !strcmp(s + a - b, suffix);
}

static int starts_with(const char *s, const char *prefix)
{
    return !strncmp(s, prefix, strlen(prefix));
}

const struct zg01_user_control *zg01_user_control_by_name(const char *name)
{
    size_t i;

    for (i = 0; i < sizeof(zg01_user_controls) / sizeof(zg01_user_controls[0]);
         i++) {
        if (!strcmp(zg01_user_controls[i].name, name))
            return &zg01_user_controls[i];
    }
    return NULL;
}

const struct zg01_user_control *
zg01_user_control_by_address(unsigned int address)
{
    size_t i;

    for (i = 0; i < sizeof(zg01_user_controls) / sizeof(zg01_user_controls[0]);
         i++) {
        if (zg01_user_controls[i].address == address)
            return &zg01_user_controls[i];
    }
    return NULL;
}

/*
 * The effect chain adds 0.1 dB controls that do not start with "EQ Band":
 * the voice output levels ("... Out Level"), the voice tone gains, the
 * game EQ M/S gains and the ten-band game EQ gains.  The dynamics "Out
 * Gain" and the de-esser gains are raw device steps, so they stay
 * integers (docs/EFFECTS_CONTROL_PLAN.md).
 */
static int is_gain(const char *name)
{
    if (ends_with(name, "Out Level"))
        return 1;
    if (!ends_with(name, "Gain"))
        return 0;
    if (ends_with(name, "Out Gain"))
        return 0;
    if (starts_with(name, "Game De-esser"))
        return 0;
    return 1;
}

/*
 * Only the microphone EQ and the ten-band game EQ carry Hz.  The tone
 * control, sound effect, de-esser and game EQ M/S frequencies are
 * indices on the vendor's 1/12-octave grid, so they must stay integers.
 */
static int is_frequency(const char *name)
{
    if (!ends_with(name, "Frequency"))
        return 0;
    return starts_with(name, "EQ Band ") ||
           starts_with(name, "Game EQ Band ");
}

static enum kind kind_of(const struct snd_ctl_elem_info *info)
{
    const char *name = (const char *)info->id.name;
    const struct zg01_user_control *ctl;

    if (info->type == SNDRV_CTL_ELEM_TYPE_BOOLEAN)
        return KIND_BOOL;

    ctl = zg01_user_control_by_name(name);
    if (ctl) {
        switch (ctl->conv) {
        case CONV_GAIN:
            return KIND_GAIN;
        case CONV_FREQ:
            return KIND_FREQ;
        case CONV_Q:
            return KIND_Q;
        default:
            return KIND_INT;
        }
    }

    /* Not in the map: the action controls and anything new. */
    if (is_gain(name))
        return KIND_GAIN;
    if (is_frequency(name))
        return KIND_FREQ;
    if (ends_with(name, "Q"))
        return KIND_Q;
    return KIND_INT;
}

static void format_value(enum kind k, long v, char *out, size_t n)
{
    switch (k) {
    case KIND_BOOL:
        snprintf(out, n, "%s", v ? "on" : "off");
        break;
    case KIND_GAIN:
        snprintf(out, n, "%+.1f dB", v / 10.0);
        break;
    case KIND_Q:
        snprintf(out, n, "%.2f", v / 100.0);
        break;
    case KIND_FREQ:
        if (v >= 1000)
            snprintf(out, n, "%ld Hz (%.2f kHz)", v, v / 1000.0);
        else
            snprintf(out, n, "%ld Hz", v);
        break;
    default:
        snprintf(out, n, "%ld", v);
        break;
    }
}

static int parse_value(enum kind k, const char *s, long *out)
{
    char *end;
    double d;

    if (k == KIND_BOOL) {
        if (!strcasecmp(s, "on") || !strcasecmp(s, "true") ||
            !strcmp(s, "1")) {
            *out = 1;
            return 0;
        }
        if (!strcasecmp(s, "off") || !strcasecmp(s, "false") ||
            !strcmp(s, "0")) {
            *out = 0;
            return 0;
        }
        return -1;
    }

    d = strtod(s, &end);
    if (end == s)
        return -1;

    if (k == KIND_FREQ) {
        if (!strcasecmp(end, "k") || !strcasecmp(end, "khz"))
            d *= 1000.0;
        else if (*end && strcasecmp(end, "hz"))
            return -1;
    } else if (k == KIND_GAIN) {
        if (*end && strcasecmp(end, "db"))
            return -1;
        d *= 10.0;
    } else if (k == KIND_Q) {
        if (*end)
            return -1;
        d *= 100.0;
    } else if (*end) {
        return -1;
    }

    *out = (long)(d >= 0 ? d + 0.5 : d - 0.5);
    return 0;
}

static int elem_info(int fd, const struct snd_ctl_elem_id *id,
                     struct snd_ctl_elem_info *info)
{
    memset(info, 0, sizeof(*info));
    info->id = *id;
    if (ioctl(fd, SNDRV_CTL_IOCTL_ELEM_INFO, info) < 0)
        return -1;
    return 0;
}

static int elem_read(int fd, const struct snd_ctl_elem_id *id, long *value)
{
    struct snd_ctl_elem_value val;

    memset(&val, 0, sizeof(val));
    val.id = *id;
    if (ioctl(fd, SNDRV_CTL_IOCTL_ELEM_READ, &val) < 0)
        return -1;
    *value = val.value.integer.value[0];
    return 0;
}

static int elem_write(int fd, const struct snd_ctl_elem_id *id, long value)
{
    struct snd_ctl_elem_value val;

    memset(&val, 0, sizeof(val));
    val.id = *id;
    val.value.integer.value[0] = value;
    if (ioctl(fd, SNDRV_CTL_IOCTL_ELEM_WRITE, &val) < 0)
        return -1;
    return 0;
}

static int list_elems(int fd)
{
    struct snd_ctl_elem_list list;

    memset(&list, 0, sizeof(list));
    if (ioctl(fd, SNDRV_CTL_IOCTL_ELEM_LIST, &list) < 0)
        return -1;

    if (list.count == 0) {
        printf("no controls\n");
        return 0;
    }

    list.pids = calloc(list.count, sizeof(struct snd_ctl_elem_id));
    if (!list.pids)
        return -1;
    list.space = list.count;

    if (ioctl(fd, SNDRV_CTL_IOCTL_ELEM_LIST, &list) < 0) {
        free(list.pids);
        return -1;
    }

    for (unsigned int i = 0; i < list.used; i++) {
        struct snd_ctl_elem_info info;
        enum kind k;
        long value;
        char text[64];

        if (elem_info(fd, &list.pids[i], &info) < 0)
            continue;
        k = kind_of(&info);
        if (elem_read(fd, &list.pids[i], &value) < 0)
            value = 0;
        format_value(k, value, text, sizeof(text));
        printf("numid=%-3u %-28s %s\n", list.pids[i].numid,
               (const char *)info.id.name, text);
    }

    free(list.pids);
    return 0;
}

static int find_by_name(int fd, const char *name, struct snd_ctl_elem_id *out)
{
    struct snd_ctl_elem_list list;

    memset(&list, 0, sizeof(list));
    if (ioctl(fd, SNDRV_CTL_IOCTL_ELEM_LIST, &list) < 0)
        return -1;
    if (list.count == 0)
        return -1;

    list.pids = calloc(list.count, sizeof(struct snd_ctl_elem_id));
    if (!list.pids)
        return -1;
    list.space = list.count;
    if (ioctl(fd, SNDRV_CTL_IOCTL_ELEM_LIST, &list) < 0) {
        free(list.pids);
        return -1;
    }

    for (unsigned int i = 0; i < list.used; i++) {
        if (!strcmp((const char *)list.pids[i].name, name)) {
            *out = list.pids[i];
            free(list.pids);
            return 0;
        }
    }

    free(list.pids);
    return -1;
}

static int cmd_get(int fd, const char *name)
{
    struct snd_ctl_elem_id id;
    struct snd_ctl_elem_info info;
    enum kind k;
    long value;
    char text[64];

    if (find_by_name(fd, name, &id) < 0) {
        fprintf(stderr, "%s: control not found: %s\n", progname, name);
        return 1;
    }
    if (elem_info(fd, &id, &info) < 0 || elem_read(fd, &id, &value) < 0) {
        fprintf(stderr, "%s: cannot read %s: %s\n", progname, name,
                strerror(errno));
        return 1;
    }

    k = kind_of(&info);
    format_value(k, value, text, sizeof(text));
    printf("%s: %s\n", name, text);
    return 0;
}

static int cmd_set(int fd, const char *name, const char *raw)
{
    struct snd_ctl_elem_id id;
    struct snd_ctl_elem_info info;
    enum kind k;
    long value;

    if (find_by_name(fd, name, &id) < 0) {
        fprintf(stderr, "%s: control not found: %s\n", progname, name);
        return 1;
    }
    if (elem_info(fd, &id, &info) < 0) {
        fprintf(stderr, "%s: cannot query %s: %s\n", progname, name,
                strerror(errno));
        return 1;
    }

    k = kind_of(&info);
    if (parse_value(k, raw, &value) < 0) {
        fprintf(stderr, "%s: invalid value for %s: %s\n", progname, name,
                raw);
        return 1;
    }
    if (value < info.value.integer.min || value > info.value.integer.max) {
        fprintf(stderr, "%s: %s out of range [%ld, %ld]\n", progname, raw,
                info.value.integer.min, info.value.integer.max);
        return 1;
    }

    if (elem_write(fd, &id, value) < 0) {
        fprintf(stderr, "%s: cannot write %s: %s\n", progname, name,
                strerror(errno));
        return 1;
    }

    return 0;
}

static int cmd_action(int fd, const char *control)
{
    struct snd_ctl_elem_id id;

    if (find_by_name(fd, control, &id) < 0) {
        fprintf(stderr, "%s: '%s' control not found\n", progname, control);
        return 1;
    }
    if (elem_write(fd, &id, 1) < 0) {
        fprintf(stderr, "%s: %s failed: %s\n", progname, control,
                strerror(errno));
        return 1;
    }
    return 0;
}

static const char *preset_open_error(int error)
{
    if (error == -EINVAL)
        return "not a valid .zgpreset file";
    if (error == -ENOTSUP)
        return "Yamaha .zg01 backups are not supported; "
               "only .zgpreset files can be read";
    return strerror(-error);
}

static void print_preset_param(const struct zgpreset_param *param)
{
    const struct zg01_user_control *ctl =
        zg01_user_control_by_address(param->address);

    printf("0x%03x %s = %u\n", (unsigned int)param->address,
           ctl ? ctl->name : "unknown", param->value);
}

/*
 * Trim, collapse whitespace and uppercase a block label so that
 * "  monitor   eq " and "MONITOR EQ" compare equal.
 */
static int normalize_block(const char *text, char *out, size_t size)
{
    size_t used = 0;
    int pending_space = 0;

    while (isspace((unsigned char)*text))
        text++;

    for (; *text; text++) {
        if (isspace((unsigned char)*text)) {
            pending_space = 1;
            continue;
        }
        if (pending_space && used) {
            if (used + 1 >= size)
                return -1;
            out[used++] = ' ';
        }
        pending_space = 0;
        if (used + 1 >= size)
            return -1;
        out[used++] = (char)toupper((unsigned char)*text);
    }

    out[used] = '\0';
    return 0;
}

static int preset_block_matches(const struct zgpreset *preset,
                                const char *want)
{
    char have[sizeof(preset->block)];
    char requested[sizeof(preset->block)];

    if (normalize_block(zgpreset_block(preset), have, sizeof(have)) < 0)
        return 0;
    if (normalize_block(want, requested, sizeof(requested)) < 0)
        return 0;
    return !strcmp(have, requested);
}

/* Device value to the control's user unit, as the driver defines it. */
static int preset_user_value(const struct zg01_user_control *ctl,
                             unsigned int value, long *user)
{
    switch (ctl->conv) {
    case CONV_GAIN:
        *user = (long)value - 180;
        break;
    case CONV_FREQ:
        *user = (long)value;
        break;
    case CONV_Q:
        if (value > ZG01_Q_DEV_MAX)
            return -1;
        *user = zg01_q_user_at_least(value);
        break;
    default:
        *user = (long)value;
        break;
    }
    return 0;
}

enum preset_status {
    PRESET_APPLY,
    PRESET_UNSUPPORTED,
    PRESET_OUT_OF_RANGE,
    PRESET_RAW,
};

struct preset_plan_entry {
    const struct zgpreset_param *param;
    const struct zg01_user_control *ctl;
    long user;
    enum preset_status status;
};

static enum preset_status preset_status_of(const struct zg01_user_control *ctl,
                                           unsigned int value, int allow_raw,
                                           long *user)
{
    if (!ctl || (ctl->flags & ZG01_CTL_READONLY))
        return PRESET_UNSUPPORTED;
    if (preset_user_value(ctl, value, user) < 0 ||
        *user < ctl->min || *user > ctl->max)
        return PRESET_OUT_OF_RANGE;
    if ((ctl->flags & ZG01_CTL_RAW) && !allow_raw)
        return PRESET_RAW;
    return PRESET_APPLY;
}

static const char *preset_status_text(enum preset_status status)
{
    switch (status) {
    case PRESET_UNSUPPORTED:
        return "unsupported, skipped";
    case PRESET_OUT_OF_RANGE:
        return "out_of_range, skipped";
    case PRESET_RAW:
        return "raw, skipped";
    default:
        return "apply";
    }
}

static void print_preset_plan_entry(const struct preset_plan_entry *entry)
{
    printf("0x%03x %s = %ld (%s)\n", (unsigned int)entry->param->address,
           entry->ctl ? entry->ctl->name : "unknown", entry->user,
           preset_status_text(entry->status));
}

static int cmd_preset_load(int argc, char **argv, const char *card)
{
    const char *path;
    const char *block = NULL;
    struct zgpreset *preset = NULL;
    struct preset_plan_entry *plan = NULL;
    char dev[64];
    int dry_run = 0;
    int allow_raw = 0;
    int applied = 0;
    int skipped = 0;
    int failed = 0;
    int fd = -1;
    int index;
    int error;
    int arg;
    size_t i;

    if (argc < 4)
        goto usage;
    path = NULL;

    /* Accept the file before or after the options. */
    for (arg = 3; arg < argc; arg++) {
        if (!strcmp(argv[arg], "--dry-run")) {
            dry_run = 1;
        } else if (!strcmp(argv[arg], "--allow-raw")) {
            allow_raw = 1;
        } else if (!strcmp(argv[arg], "--block")) {
            if (++arg >= argc)
                goto usage;
            block = argv[arg];
        } else if (!path) {
            path = argv[arg];
        } else {
            goto usage;
        }
    }
    if (!path)
        goto usage;

    error = zgpreset_open(path, &preset);
    if (error) {
        fprintf(stderr, "%s: cannot parse %s: %s\n", progname, path,
                preset_open_error(error));
        return 2;
    }

    if (block && !preset_block_matches(preset, block)) {
        fprintf(stderr, "%s: preset block '%s' does not match --block '%s'\n",
                progname, zgpreset_block(preset), block);
        zgpreset_close(preset);
        return 2;
    }

    /*
     * Resolve and convert every parameter before opening the card, so a
     * dry-run and a real load share one plan and a real load never writes
     * a value the dry-run would have rejected.
     */
    plan = calloc(preset->count ? preset->count : 1, sizeof(*plan));
    if (!plan) {
        fprintf(stderr, "%s: out of memory\n", progname);
        zgpreset_close(preset);
        return 2;
    }

    for (i = 0; i < preset->count; i++) {
        plan[i].param = &preset->params[i];
        plan[i].ctl = zg01_user_control_by_address(plan[i].param->address);
        plan[i].user = (long)plan[i].param->value;
        plan[i].status = preset_status_of(plan[i].ctl, plan[i].param->value,
                                          allow_raw, &plan[i].user);
    }

    if (dry_run) {
        for (i = 0; i < preset->count; i++) {
            print_preset_plan_entry(&plan[i]);
            if (plan[i].status != PRESET_APPLY)
                skipped = 1;
        }
        free(plan);
        zgpreset_close(preset);
        return skipped ? 1 : 0;
    }

    index = find_card(card);
    if (index < 0) {
        fprintf(stderr, "%s: no ZG01 card found\n", progname);
        free(plan);
        zgpreset_close(preset);
        return 1;
    }

    snprintf(dev, sizeof(dev), "/dev/snd/controlC%d", index);
    fd = open(dev, O_RDWR);
    if (fd < 0) {
        fprintf(stderr, "%s: cannot open %s: %s\n", progname, dev,
                strerror(errno));
        free(plan);
        zgpreset_close(preset);
        return 1;
    }

    for (i = 0; i < preset->count; i++) {
        struct preset_plan_entry *entry = &plan[i];
        struct snd_ctl_elem_id id;

        if (entry->status != PRESET_APPLY) {
            print_preset_plan_entry(entry);
            skipped++;
            continue;
        }

        if (find_by_name(fd, entry->ctl->name, &id) < 0) {
            printf("0x%03x %s = %ld (failed: control not found)\n",
                   (unsigned int)entry->param->address, entry->ctl->name,
                   entry->user);
            failed++;
            continue;
        }
        if (elem_write(fd, &id, entry->user) < 0) {
            int saved = errno;

            printf("0x%03x %s = %ld (failed: %s)\n",
                   (unsigned int)entry->param->address, entry->ctl->name,
                   entry->user, strerror(saved));
            failed++;
            continue;
        }

        printf("0x%03x %s = %ld (applied)\n",
               (unsigned int)entry->param->address, entry->ctl->name,
               entry->user);
        applied++;
    }

    printf("applied=%d skipped=%d failed=%d\n", applied, skipped, failed);

    close(fd);
    free(plan);
    zgpreset_close(preset);
    return (skipped || failed) ? 1 : 0;

usage:
    usage();
    return 2;
}

static int cmd_preset(int argc, char **argv, const char *card)
{
    struct zgpreset *preset = NULL;
    int show;
    size_t i;
    int error;

    if (argc < 3)
        goto usage;
    if (!strcmp(argv[2], "load"))
        return cmd_preset_load(argc, argv, card);

    show = !strcmp(argv[2], "show");
    if (!show && strcmp(argv[2], "list"))
        goto usage;
    if (argc != 4)
        goto usage;

    error = zgpreset_open(argv[3], &preset);
    if (error) {
        fprintf(stderr, "%s: cannot parse %s: %s\n", progname, argv[3],
                preset_open_error(error));
        return 2;
    }

    printf("block: %s\n", zgpreset_block(preset));
    printf("name: %s\n", zgpreset_name(preset));
    if (show) {
        for (i = 0; i < preset->count; i++)
            print_preset_param(&preset->params[i]);
    } else {
        printf("params: %zu\n", preset->count);
    }

    zgpreset_close(preset);
    return 0;

usage:
    usage();
    return 2;
}

int main(int argc, char **argv)
{
    const char *card = NULL;
    const char *cmd;
    char dev[64];
    int index;
    int fd;

    if (argc > 1 && (!strcmp(argv[1], "-h") || !strcmp(argv[1], "--help"))) {
        usage();
        return 0;
    }

    if (argc > 2 && !strcmp(argv[1], "-c")) {
        card = argv[2];
        argc -= 2;
        argv += 2;
    }

    if (argc < 2) {
        usage();
        return 2;
    }
    cmd = argv[1];

    /* Reading a preset file never touches the card. */
    if (!strcmp(cmd, "preset"))
        return cmd_preset(argc, argv, card);

    index = find_card(card);
    if (index < 0) {
        fprintf(stderr, "%s: no ZG01 card found\n", progname);
        return 1;
    }

    snprintf(dev, sizeof(dev), "/dev/snd/controlC%d", index);
    fd = open(dev, O_RDWR);
    if (fd < 0) {
        fprintf(stderr, "%s: cannot open %s: %s\n", progname, dev,
                strerror(errno));
        return 1;
    }

    if (!strcmp(cmd, "list")) {
        if (argc != 2) {
            usage();
            close(fd);
            return 2;
        }
        index = list_elems(fd);
        close(fd);
        return index < 0 ? 1 : 0;
    }
    if (!strcmp(cmd, "get") && argc == 3) {
        index = cmd_get(fd, argv[2]);
        close(fd);
        return index;
    }
    if (!strcmp(cmd, "set") && argc == 4) {
        index = cmd_set(fd, argv[2], argv[3]);
        close(fd);
        return index;
    }
    if (!strcmp(cmd, "save") && argc == 2) {
        index = cmd_action(fd, "Save to ZG01");
        close(fd);
        return index;
    }
    if (!strcmp(cmd, "reset") && argc == 2) {
        index = cmd_action(fd, "Reset to ZG01");
        close(fd);
        return index;
    }

    usage();
    close(fd);
    return 2;
}
