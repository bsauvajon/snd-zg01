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
 */

#include <ctype.h>
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>
#include <sys/ioctl.h>
#include <unistd.h>

#include <sound/asound.h>

enum kind {
    KIND_INT,
    KIND_BOOL,
    KIND_GAIN,   /* value is 0.1 dB */
    KIND_FREQ,   /* value is Hz */
    KIND_Q,      /* value is 0.01 Q */
};

static const char *progname = "zgctl";

static void usage(void)
{
    fprintf(stderr,
            "usage: %s [-c CARD] list\n"
            "       %s [-c CARD] get CONTROL\n"
            "       %s [-c CARD] set CONTROL VALUE\n"
            "\n"
            "CARD defaults to the card whose driver is zg01_usb.\n"
            "VALUES: Gain in dB (+6.0), Frequency in Hz (1k, 7.5kHz),\n"
            "        Q as a ratio (2.0), switches on/off.\n",
            progname, progname, progname);
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

static enum kind kind_of(const struct snd_ctl_elem_info *info)
{
    const char *name = (const char *)info->id.name;

    if (info->type == SNDRV_CTL_ELEM_TYPE_BOOLEAN)
        return KIND_BOOL;
    if (ends_with(name, "Gain"))
        return KIND_GAIN;
    if (ends_with(name, "Frequency"))
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

    usage();
    close(fd);
    return 2;
}
