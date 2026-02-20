#ifndef ZG01_PCM_H
#define ZG01_PCM_H

#include <sound/pcm.h>

/* Forward declaration */
struct zg01_dev;

struct zg01_pcm {
    struct zg01_dev *zg01;
    struct snd_pcm *instance;
};

#endif /* ZG01_PCM_H */