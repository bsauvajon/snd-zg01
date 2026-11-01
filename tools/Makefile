CC ?= gcc
CFLAGS ?= -O2 -Wall -Wextra

all: zgctl

zgctl: zgctl.c zgpreset.c zgpreset.h zg01_controls_generated.h ../src/zg01_codec.h
	$(CC) $(CFLAGS) -o $@ zgctl.c zgpreset.c

clean:
	rm -f zgctl

.PHONY: all clean
