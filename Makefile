# Makefile for Yamaha ZG01 USB audio driver

# Single module from all source files
obj-m := snd-zg01.o
snd-zg01-objs := src/zg01_usb.o src/zg01_pcm.o src/zg01_control.o src/zg01_usb_discovery.o

KDIR ?= /lib/modules/$(shell uname -r)/build

all:
	$(MAKE) -C $(KDIR) M=$(PWD) modules

clean:
	$(MAKE) -C $(KDIR) M=$(PWD) clean
