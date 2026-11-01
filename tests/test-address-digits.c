/*
 * Host contract for the address macros the driver puts on the wire.
 *
 * The test used to recompute the split in Python, which could pass while
 * the driver's macros used a different divisor.  This compiles the actual
 * production header, including the page-0 0x7f/0x80 boundary.
 */
#include <assert.h>
#include <stdio.h>

#include "zg01_addr.h"

int main(void)
{
    unsigned int addr;

    /* Page 0 low half: high digit 0, low digit is the offset. */
    assert(ZG01_ADDR_FLAG(0x003) == 0x00);
    assert(ZG01_ADDR_ID(0x003) == 0x03);

    /* Boundary: 0x7f is the last low-half offset, 0x80 the first high. */
    assert(ZG01_ADDR_FLAG(0x07f) == 0x00);
    assert(ZG01_ADDR_ID(0x07f) == 0x7f);
    assert(ZG01_ADDR_FLAG(0x080) == 0x01);
    assert(ZG01_ADDR_ID(0x080) == 0x00);

    /* Page 0 monitor volume and page 1 EQ band 1 gain. */
    assert(ZG01_ADDR_FLAG(0x0e9) == 0x01);
    assert(ZG01_ADDR_ID(0x0e9) == 0x69);
    assert(ZG01_ADDR_FLAG(0x128) == 0x02);
    assert(ZG01_ADDR_ID(0x128) == 0x28);

    /* Every address the two pages can carry round-trips through 7 bits. */
    for (addr = 0; addr < 2 * 256; addr++) {
        unsigned int flag = ZG01_ADDR_FLAG(addr);
        unsigned int id = ZG01_ADDR_ID(addr);

        assert(flag <= 0x7f);
        assert(id <= 0x7f);
        assert(flag * 128 + id == addr);
    }

    puts("address digit contract passed");
    return 0;
}
