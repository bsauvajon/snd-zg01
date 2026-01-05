# Makefile for Yamaha ZG01 USB audio driver

# Kernel build environment
KDIR := /lib/modules/$(shell uname -r)/build

# Object files (in src/ directory)
obj-m := src/zg01_usb.o src/zg01_pcm.o src/zg01_control.o src/zg01_usb_discovery.o

# Default rule
all:
	$(MAKE) -C $(KDIR) M=$(PWD) modules

# Clean rule
clean:
	$(MAKE) -C $(KDIR) M=$(PWD) clean
