#ifndef ZG01_CONTROL_H
#define ZG01_CONTROL_H

struct zg01_dev;

struct zg01_control {
	struct zg01_dev *zg01;

	bool phono_mic_switch;
};

int zg01_init_control(struct zg01_dev *zg01);

#endif /* ZG01_CONTROL_H */