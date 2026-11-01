/*
 * zgpreset - read the MBDF container and values in Yamaha ZG01 preset files.
 *
 * This is the C port of tools/zgpreset.py.  Only .zgpreset files are
 * supported.  A file starts with a fixed 0x74-byte header whose magic is
 * "#YAMAHA MBDFPreset".  Real files put a 36-byte file identifier between
 * that header and the chunk stream; a '#' at offset 0x74 means the
 * identifier is absent, as in synthetic fixtures.
 *
 * Chunks and components are validated conservatively: a length that runs
 * past the end of its container, an unknown component magic, an unknown
 * value width and trailing bytes that do not form a whole record are all
 * rejected.
 *
 * An F_* component is a schema.  The 4-byte marker 41 01 00 80 ends its
 * header; the address/marker/width records follow it.  Its paired D_*
 * component carries a display name and the little-endian values.  The
 * display-name field differs in length per block, so the value blob starts
 * at len(D) - sum(widths).
 *
 * A .zg01 backup uses the same container but is not parsed: its multi-group
 * F_CurrentScene schema is a follow-up.  zgpreset_open returns -ENOTSUP for
 * the backup magic so every caller reports a clear "backup not supported"
 * error instead of claiming a decode it cannot do.
 *
 * Build:  make -C tools
 */

#include "zgpreset.h"

#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <unistd.h>

#define HEADER_SIZE 0x74u
#define FILE_ID_SIZE 36u
#define CHUNK_HEADER_SIZE 36u
#define COMPONENT_ENVELOPE_SIZE 8u
#define COMPONENT_HEADER_SIZE 32u
#define COMPONENT_NAME_SIZE 24u
#define SCHEMA_MARKER_SIZE 4u
#define SCHEMA_RECORD_SIZE 4u
#define SCHEMA_RECORD_MARKER 0x01u

#define COMPONENT_MAGIC_PRESET 0xda27u
#define COMPONENT_MAGIC_MONITOR 0xd927u

struct chunk {
    char tag[9];
    char name[13];
    const unsigned char *payload;
    size_t length;
};

struct component {
    char name[COMPONENT_NAME_SIZE + 1];
    const unsigned char *payload;
    size_t length;
};

static uint16_t read_be16(const unsigned char *bytes)
{
    return (uint16_t)(((uint16_t)bytes[0] << 8) | bytes[1]);
}

static uint16_t read_le16(const unsigned char *bytes)
{
    return (uint16_t)(((uint16_t)bytes[1] << 8) | bytes[0]);
}

static uint64_t read_be64(const unsigned char *bytes)
{
    uint64_t value = 0;
    size_t i;

    for (i = 0; i < 8; i++)
        value = (value << 8) | bytes[i];
    return value;
}

/* Decode a fixed-size NUL-padded field into a bounded C string. */
static void field_text(char *dst, size_t size, const char *src, size_t length)
{
    size_t i = 0;

    while (i < length && i + 1 < size && src[i] != '\0') {
        dst[i] = src[i];
        i++;
    }
    dst[i] = '\0';
}

static int starts_with(const char *text, const char *prefix)
{
    return strncmp(text, prefix, strlen(prefix)) == 0;
}

/*
 * Grow *capacity so it can hold at least "needed" items.  On success the
 * (possibly moved) allocation is returned through grown; the caller keeps
 * ownership of the old pointer and assigns the new one.
 */
static int grow_array(void *items, size_t *capacity, size_t needed,
                      size_t item_size, void **grown)
{
    size_t new_capacity;
    void *resized;

    if (needed <= *capacity) {
        *grown = items;
        return 0;
    }

    new_capacity = *capacity ? *capacity : 16;
    while (new_capacity < needed) {
        if (new_capacity > SIZE_MAX / 2) {
            new_capacity = needed;
            break;
        }
        new_capacity *= 2;
    }
    if (new_capacity > SIZE_MAX / item_size)
        return -ENOMEM;

    resized = realloc(items, new_capacity * item_size);
    if (!resized)
        return -ENOMEM;

    *grown = resized;
    *capacity = new_capacity;
    return 0;
}

static int read_file(const char *path, unsigned char **data_out,
                     size_t *size_out)
{
    struct stat info;
    unsigned char *data;
    size_t size;
    size_t done = 0;
    int fd;
    int error;

    fd = open(path, O_RDONLY);
    if (fd < 0)
        return -errno;

    if (fstat(fd, &info) != 0) {
        error = errno;
        close(fd);
        return -error;
    }
    if (info.st_size < 0) {
        close(fd);
        return -EINVAL;
    }
    size = (size_t)info.st_size;
    if ((off_t)size != info.st_size) {
        close(fd);
        return -EINVAL;
    }

    data = malloc(size ? size : 1);
    if (!data) {
        close(fd);
        return -ENOMEM;
    }

    while (done < size) {
        ssize_t got = read(fd, data + done, size - done);

        if (got < 0) {
            if (errno == EINTR)
                continue;
            error = errno;
            free(data);
            close(fd);
            return -error;
        }
        if (got == 0) {
            free(data);
            close(fd);
            return -EIO;
        }
        done += (size_t)got;
    }

    close(fd);
    *data_out = data;
    *size_out = size;
    return 0;
}

/* Locate the chunk stream after the fixed header and optional file id. */
static int chunk_stream_start(const unsigned char *data, size_t size,
                              size_t *start)
{
    static const char preset_magic[] = "#YAMAHA MBDFPreset";
    static const char backup_magic[] = "#YAMAHA MBDFBackup";

    if (size < HEADER_SIZE)
        return -EINVAL;
    if (memcmp(data, backup_magic, sizeof(backup_magic) - 1) == 0)
        return -ENOTSUP;
    if (memcmp(data, preset_magic, sizeof(preset_magic) - 1) != 0)
        return -EINVAL;

    *start = HEADER_SIZE;
    /* A '#' at the header boundary is the start of a chunk tag, so the file
     * identifier is absent; real files always carry it. */
    if (*start < size && data[*start] != '#' && size - *start >= FILE_ID_SIZE)
        *start += FILE_ID_SIZE;
    return 0;
}

static int parse_chunks(const unsigned char *data, size_t size,
                        struct chunk **chunks_out, size_t *count_out)
{
    struct chunk *chunks = NULL;
    size_t count = 0;
    size_t capacity = 0;
    size_t offset;
    int error;

    error = chunk_stream_start(data, size, &offset);
    if (error)
        return error;

    while (offset < size) {
        struct chunk entry;
        size_t remaining = size - offset;
        uint64_t length;
        void *grown;

        if (remaining < CHUNK_HEADER_SIZE) {
            error = -EINVAL;
            goto fail;
        }

        field_text(entry.tag, sizeof(entry.tag),
                   (const char *)data + offset, 8);
        field_text(entry.name, sizeof(entry.name),
                   (const char *)data + offset + 12, 12);
        length = read_be64(data + offset + 24);
        if (length > (uint64_t)(remaining - CHUNK_HEADER_SIZE)) {
            error = -EINVAL;
            goto fail;
        }
        entry.payload = data + offset + CHUNK_HEADER_SIZE;
        entry.length = (size_t)length;

        error = grow_array(chunks, &capacity, count + 1, sizeof(*chunks),
                           &grown);
        if (error)
            goto fail;
        chunks = grown;
        chunks[count++] = entry;
        offset += CHUNK_HEADER_SIZE + entry.length;
    }

    *chunks_out = chunks;
    *count_out = count;
    return 0;

fail:
    free(chunks);
    return error;
}

static int parse_components(const struct chunk *chunk,
                            struct component **components_out,
                            size_t *count_out, size_t *capacity)
{
    const unsigned char *payload = chunk->payload;
    size_t length = chunk->length;
    size_t offset = 0;

    if (length >= COMPONENT_ENVELOPE_SIZE &&
        memcmp(payload, "Comp\0\0\0\0", COMPONENT_ENVELOPE_SIZE) == 0) {
        payload += COMPONENT_ENVELOPE_SIZE;
        length -= COMPONENT_ENVELOPE_SIZE;
    }

    while (offset < length) {
        struct component entry;
        size_t remaining = length - offset;
        uint16_t magic;
        uint16_t payload_length;
        void *grown;
        int error;

        if (remaining < COMPONENT_HEADER_SIZE)
            return -EINVAL;

        magic = read_be16(payload + offset + 24);
        if (magic != COMPONENT_MAGIC_PRESET &&
            magic != COMPONENT_MAGIC_MONITOR)
            return -EINVAL;

        payload_length = read_le16(payload + offset + 28);
        if (payload_length == 0)
            return -EINVAL;
        if ((size_t)payload_length >
            remaining - COMPONENT_HEADER_SIZE)
            return -EINVAL;

        field_text(entry.name, sizeof(entry.name),
                   (const char *)payload + offset, COMPONENT_NAME_SIZE);
        entry.payload = payload + offset + COMPONENT_HEADER_SIZE;
        entry.length = payload_length;

        error = grow_array(*components_out, capacity, *count_out + 1,
                           sizeof(**components_out), &grown);
        if (error)
            return error;
        *components_out = grown;
        (*components_out)[(*count_out)++] = entry;
        offset += COMPONENT_HEADER_SIZE + entry.length;
    }
    return 0;
}

static int chunk_is_data(const struct chunk *chunk)
{
    if (!starts_with(chunk->tag, "#PRESET"))
        return 0;
    return strcmp(chunk->name, "ChunkData") == 0 ||
           strcmp(chunk->name, "DATA") == 0;
}

static int parse_info_payload(const unsigned char *payload, size_t length,
                              char block[32], char **name_out)
{
    size_t name_length;
    size_t i;
    char *name;

    block[0] = '\0';
    *name_out = NULL;

    if (length < 64) {
        field_text(block, 32, (const char *)payload, length);
        return 0;
    }
    field_text(block, 32, (const char *)payload, 64);
    /* The display name follows a 0x02 marker at offset 80 in every real
     * #PRESET General chunk; the 16 bytes before it are block metadata.
     * Allocate the whole field: a fixed-size copy both truncated long
     * names and could cut a multi-byte UTF-8 sequence in half. */
    if (length <= 80 || payload[80] != 0x02)
        return 0;

    name_length = length - 81;
    name = malloc(name_length + 1);
    if (!name)
        return -ENOMEM;

    for (i = 0; i < name_length && payload[81 + i] != '\0'; i++)
        name[i] = (char)payload[81 + i];
    name[i] = '\0';
    *name_out = name;
    return 0;
}

static int preset_info(const struct chunk *chunks, size_t count,
                       char block[32], char **name_out)
{
    size_t i;

    *name_out = NULL;
    for (i = 0; i < count; i++) {
        if (starts_with(chunks[i].tag, "#PRESET") &&
            (strcmp(chunks[i].name, "General") == 0 ||
             strcmp(chunks[i].name, "INFOGeneral") == 0))
            return parse_info_payload(chunks[i].payload, chunks[i].length,
                                      block, name_out);
    }

    block[0] = '\0';
    return 0;
}

/* The Python reference keeps the last D_* component for a suffix. */
static const struct component *find_values(const struct component *components,
                                           size_t count,
                                           const char *schema_name)
{
    const struct component *found = NULL;
    const char *suffix = schema_name + 2;
    size_t i;

    for (i = 0; i < count; i++) {
        if (strncmp(components[i].name, "D_", 2) == 0 &&
            strcmp(components[i].name + 2, suffix) == 0)
            found = &components[i];
    }
    return found;
}

static const unsigned char *find_schema_marker(const unsigned char *payload,
                                               size_t length)
{
    static const unsigned char marker[SCHEMA_MARKER_SIZE] = {
        0x41, 0x01, 0x00, 0x80,
    };
    size_t offset;

    if (length < SCHEMA_MARKER_SIZE)
        return NULL;
    for (offset = 0; offset <= length - SCHEMA_MARKER_SIZE; offset++) {
        if (memcmp(payload + offset, marker, SCHEMA_MARKER_SIZE) == 0)
            return payload + offset;
    }
    return NULL;
}

static int decode_component(const struct component *schema,
                            const struct component *values,
                            struct zgpreset_param **params, size_t *count,
                            size_t *capacity)
{
    const unsigned char *marker;
    const unsigned char *body;
    size_t body_length;
    size_t records;
    size_t total_width = 0;
    size_t value_offset;
    size_t offset;
    void *grown;
    int error;

    marker = find_schema_marker(schema->payload, schema->length);
    if (!marker)
        return -EINVAL;

    body = marker + SCHEMA_MARKER_SIZE;
    body_length = schema->length - (size_t)(body - schema->payload);
    if (body_length % SCHEMA_RECORD_SIZE)
        return -EINVAL;
    records = body_length / SCHEMA_RECORD_SIZE;

    for (offset = 0; offset < body_length; offset += SCHEMA_RECORD_SIZE) {
        unsigned int width = body[offset + 3];

        if (body[offset + 2] != SCHEMA_RECORD_MARKER ||
            (width != 1 && width != 2))
            return -EINVAL;
        total_width += width;
    }
    if (total_width > values->length)
        return -EINVAL;
    value_offset = values->length - total_width;

    if (records) {
        if (records > SIZE_MAX - *count)
            return -ENOMEM;
        error = grow_array(*params, capacity, *count + records,
                           sizeof(**params), &grown);
        if (error)
            return error;
        *params = grown;
    }

    for (offset = 0; offset < body_length; offset += SCHEMA_RECORD_SIZE) {
        struct zgpreset_param *param = &(*params)[*count];
        unsigned int width = body[offset + 3];

        param->address = (uint16_t)(((unsigned int)body[offset] << 8) |
                                    body[offset + 1]);
        param->width = (uint8_t)width;
        if (width == 1)
            param->value = values->payload[value_offset];
        else
            param->value = read_le16(values->payload + value_offset);
        value_offset += width;
        (*count)++;
    }
    return 0;
}

int zgpreset_open(const char *path, struct zgpreset **out)
{
    unsigned char *data = NULL;
    size_t size = 0;
    struct chunk *chunks = NULL;
    size_t chunk_count = 0;
    struct component *components = NULL;
    size_t component_count = 0;
    size_t component_capacity = 0;
    struct zgpreset *preset = NULL;
    size_t param_capacity = 0;
    size_t i;
    int error = 0;

    if (!out)
        return -EINVAL;
    *out = NULL;
    if (!path)
        return -EINVAL;

    error = read_file(path, &data, &size);
    if (error)
        goto done;

    error = parse_chunks(data, size, &chunks, &chunk_count);
    if (error)
        goto done;

    preset = calloc(1, sizeof(*preset));
    if (!preset) {
        error = -ENOMEM;
        goto done;
    }
    error = preset_info(chunks, chunk_count, preset->block, &preset->name);
    if (error)
        goto done;

    for (i = 0; i < chunk_count; i++) {
        if (!chunk_is_data(&chunks[i]))
            continue;
        error = parse_components(&chunks[i], &components, &component_count,
                                 &component_capacity);
        if (error)
            goto done;
    }

    for (i = 0; i < component_count; i++) {
        const struct component *values;

        if (strncmp(components[i].name, "F_", 2) != 0)
            continue;
        values = find_values(components, component_count, components[i].name);
        if (!values) {
            error = -EINVAL;
            goto done;
        }
        error = decode_component(&components[i], values, &preset->params,
                                 &preset->count, &param_capacity);
        if (error)
            goto done;
    }

    if (preset->count == 0) {
        error = -EINVAL;
        goto done;
    }

    *out = preset;
    preset = NULL;

done:
    free(data);
    free(chunks);
    free(components);
    zgpreset_close(preset);
    return error;
}

const char *zgpreset_block(const struct zgpreset *preset)
{
    return preset ? preset->block : "";
}

const char *zgpreset_name(const struct zgpreset *preset)
{
    return preset && preset->name ? preset->name : "";
}

void zgpreset_close(struct zgpreset *preset)
{
    if (!preset)
        return;
    free(preset->name);
    free(preset->params);
    free(preset);
}
