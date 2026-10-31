CC ?= gcc
CFLAGS ?= -O2 -Wall -Wextra

all: zgctl

zgctl: zgctl.c
	$(CC) $(CFLAGS) -o $@ $<

clean:
	rm -f zgctl

.PHONY: all clean
