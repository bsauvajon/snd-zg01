/*
 * ZG01 parameter address encoding.
 *
 * A parameter address is page * 256 + offset.  The two bytes a write frame
 * carries are the base-128 digits of that address, because a system
 * exclusive data byte holds 7 bits.  Keep the conversion in one place so a
 * host test can exercise the same macros the driver uses (including the
 * page-0 0x7f/0x80 boundary, where the high digit changes from 0 to 1).
 */
#ifndef ZG01_ADDR_H
#define ZG01_ADDR_H

#define ZG01_ADDR_FLAG(addr)  ((unsigned char)((addr) / 128))
#define ZG01_ADDR_ID(addr)    ((unsigned char)((addr) % 128))

#endif /* ZG01_ADDR_H */
