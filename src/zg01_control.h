#ifndef CONTROL_H
#define CONTROL_H

struct zg01_dev;

struct zg01_control {
	struct zg01_dev *zg01;

	bool phono_mic_switch;
};

int zg01_init_control(struct zg01_dev *zg01);
void zg01_free_control(struct zg01_dev *zg01);

#endif