# Makefile for Yamaha ZG01 USB audio driver

# Single module from all source files
obj-m := snd-zg01.o
snd-zg01-objs := src/zg01_usb.o src/zg01_pcm.o src/zg01_control.o src/zg01_usb_discovery.o

KDIR ?= /lib/modules/$(shell uname -r)/build

# CachyOS ships clang-built and gcc-built kernels side by side. The target
# kernel's build system injects compiler-specific flags, so an external
# module must use the same compiler family. Read the compiler from the
# target kernel's .config and enable the LLVM front-end only when needed.
# A user-supplied LLVM value always wins.
ifeq ($(origin LLVM),undefined)
ifeq ($(shell grep -s -q '^CONFIG_CC_IS_CLANG=y' $(KDIR)/.config && echo yes),yes)
LLVM = 1
endif
endif

all:
	$(MAKE) -C $(KDIR) M=$(PWD) $(if $(LLVM),LLVM=$(LLVM)) modules

clean:
	$(MAKE) -C $(KDIR) M=$(PWD) clean
