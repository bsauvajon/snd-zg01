# ZG01 parameter inventory

Every parameter the vendor application can drive, grouped by
screen, extracted from the MemAPI library shipped in *TOOLS for
ZG* (see `docs/VENDOR_PACKAGE.md` for the procedure and the
caveats).  `_ForBackup` mirrors are omitted; they duplicate the
same parameter for the persistence layer.

Total: **383** parameters (331 CamelCase, 52 legacy snake_case).

## Mic (GATE, COMP, EQ, LIMITER, mute, P48V) (54)

| Parameter | Min | Max | Default |
|---|---|---|---|
| `DirtyFlagMic` | 0 | 1 | 0 |
| `DirtyFlagMicFx` | 0 | 1 | 0 |
| `MicCompAttack` | 57 | 283 | 237 |
| `MicCompKnee` | 0 | 2 | 1 |
| `MicCompLevel` | 0 | 100 | 50 |
| `MicCompOnf` | 0 | 1 | 0 |
| `MicCompOutgain` | 0 | 180 | 50 |
| `MicCompRatio` | 0 | 120 | 14 |
| `MicCompRelease` | 24 | 300 | 228 |
| `MicCompThreshold` | 19 | 73 | 52 |
| `MicEffectButtonFunc1` | 0 | 2 | 0 |
| `MicEffectButtonFunc2` | 0 | 2 | 1 |
| `MicEffectButtonFunc3` | 0 | 2 | 2 |
| `MicEqName` | 0 | 255 | 0 |
| `MicEqOnf` | 0 | 1 | 0 |
| `MicEqV21F` | 20 | 1000 | 100 |
| `MicEqV21G` | 0 | 360 | 180 |
| `MicEqV21Onf` | 0 | 1 | 1 |
| `MicEqV21Q` | 0 | 60 | 12 |
| `MicEqV21Shape` | 0 | 1 | 1 |
| `MicEqV22F` | 20 | 20000 | 500 |
| `MicEqV22G` | 0 | 360 | 180 |
| `MicEqV22Onf` | 0 | 1 | 1 |
| `MicEqV22Q` | 0 | 60 | 12 |
| `MicEqV22Shape` | 0 | 1 | 0 |
| `MicEqV23F` | 20 | 20000 | 2500 |
| `MicEqV23G` | 0 | 360 | 180 |
| `MicEqV23Onf` | 0 | 1 | 1 |
| `MicEqV23Q` | 0 | 60 | 12 |
| `MicEqV23Shape` | 0 | 1 | 0 |
| `MicEqV24F` | 500 | 20000 | 10000 |
| `MicEqV24G` | 0 | 360 | 180 |
| `MicEqV24Onf` | 0 | 1 | 1 |
| `MicEqV24Q` | 0 | 60 | 12 |
| `MicEqV24Shape` | 0 | 1 | 1 |
| `MicGateAttack` | 57 | 283 | 237 |
| `MicGateDecay` | 24 | 300 | 188 |
| `MicGateHold` | 0 | 213 | 101 |
| `MicGateLevel` | 0 | 100 | 50 |
| `MicGateOnf` | 0 | 1 | 0 |
| `MicGateRange` | 0 | 73 | 17 |
| `MicGateThreshold` | 1 | 73 | 23 |
| `MicLevel` | 0 | 127 | 103 |
| `MicLimiterAttack` | 57 | 283 | 237 |
| `MicLimiterKnee` | 0 | 2 | 1 |
| `MicLimiterLevel` | 0 | 100 | 50 |
| `MicLimiterOnf` | 0 | 1 | 1 |
| `MicLimiterOutgain` | 0 | 180 | 0 |
| `MicLimiterRelease` | 24 | 300 | 228 |
| `MicLimiterThreshold` | 19 | 73 | 65 |
| `MicMonitorLevel` | 0 | 127 | 103 |
| `MicMonitorLevelKnob` | 0 | 127 | 64 |
| `MicMute` | 0 | 1 | 0 |
| `MicP48V` | 0 | 1 | 0 |

## MIC EFFECT (voice changer, SE, echo, reverb, tone control, transpose) (69)

| Parameter | Min | Max | Default |
|---|---|---|---|
| `EchoDelayHighRatio` | 1 | 10 | 8 |
| `EchoDelayLevel` | 0 | 103 | 103 |
| `EchoDelayTime` | 200 | 7430 | 2400 |
| `EchoDensity` | 0 | 4 | 4 |
| `EchoDiffusion` | 0 | 10 | 10 |
| `EchoFeedback` | 1 | 127 | 94 |
| `EchoHighRatio` | 1 | 10 | 8 |
| `EchoHpf` | 0 | 52 | 0 |
| `EchoInitDelay` | 0 | 63 | 16 |
| `EchoLpf` | 34 | 60 | 53 |
| `EchoOnf` | 0 | 1 | 0 |
| `EchoRevLevel` | 0 | 103 | 91 |
| `EchoRevTime` | 0 | 57 | 25 |
| `EchoRevType` | 0 | 4 | 0 |
| `EchoStereo` | 0 | 1 | 0 |
| `EchoTime` | 0 | 2 | 1 |
| `SeDbLevel` | 1 | 73 | 53 |
| `SeFreq` | 29 | 52 | 44 |
| `SeLevel` | 0 | 2 | 1 |
| `SeOnf` | 0 | 1 | 0 |
| `SeWaveform` | 0 | 3 | 0 |
| `VoiceEffectOnf` | 0 | 1 | 0 |
| `VoiceEffectRadioCutoff` | 50 | 113 | 61 |
| `VoiceEffectRadioDrive` | 1 | 25 | 10 |
| `VoiceEffectRadioLpfCutoff` | 34 | 60 | 50 |
| `VoiceEffectRadioOutLevel` | 0 | 360 | 180 |
| `VoiceEffectTremoloDepth` | 0 | 127 | 64 |
| `VoiceEffectTremoloOutLevel` | 0 | 360 | 180 |
| `VoiceEffectTremoloSpeed` | 1 | 127 | 89 |
| `VoiceEffectType` | 0 | 1 | 0 |
| `VoiceRevDensity` | 0 | 4 | 4 |
| `VoiceRevDiffusion` | 0 | 10 | 10 |
| `VoiceRevHighRatio` | 1 | 10 | 8 |
| `VoiceRevHpf` | 0 | 52 | 0 |
| `VoiceRevInitDelay` | 0 | 63 | 16 |
| `VoiceRevLpf` | 34 | 60 | 53 |
| `VoiceRevOnf` | 0 | 1 | 0 |
| `VoiceRevTime` | 0 | 57 | 25 |
| `VoiceRevType` | 0 | 4 | 0 |
| `VoiceRevWet` | 1 | 127 | 64 |
| `VoiceTcHiFreq` | 60 | 124 | 112 |
| `VoiceTcHiGain` | 0 | 360 | 180 |
| `VoiceTcHiOnf` | 0 | 1 | 1 |
| `VoiceTcHiQual` | 0 | 60 | 12 |
| `VoiceTcHiShape` | 0 | 1 | 1 |
| `VoiceTcLoFreq` | 4 | 72 | 32 |
| `VoiceTcLoGain` | 0 | 360 | 180 |
| `VoiceTcLoOnf` | 0 | 1 | 1 |
| `VoiceTcLoQual` | 0 | 60 | 12 |
| `VoiceTcLoShape` | 0 | 1 | 1 |
| `VoiceTcMdFreq` | 4 | 124 | 72 |
| `VoiceTcMdGain` | 0 | 360 | 180 |
| `VoiceTcMdOnf` | 0 | 1 | 1 |
| `VoiceTcMdQual` | 0 | 60 | 12 |
| `VoiceTcOnf` | 0 | 1 | 0 |
| `VoiceTransHarmCorrectSense` | 0 | 100 | 64 |
| `VoiceTransHarmFormant` | 2 | 126 | 64 |
| `VoiceTransHarmLevel` | 0 | 127 | 127 |
| `VoiceTransHarmOutLevel` | 0 | 360 | 180 |
| `VoiceTransHarmTranspose` | 28 | 100 | 64 |
| `VoiceTransOnf` | 0 | 1 | 0 |
| `VoiceTransPitchFine1` | 14 | 114 | 64 |
| `VoiceTransPitchFine2` | 14 | 114 | 64 |
| `VoiceTransPitchLevel1` | 0 | 127 | 100 |
| `VoiceTransPitchLevel2` | 0 | 127 | 100 |
| `VoiceTransPitchOutLevel` | 0 | 360 | 180 |
| `VoiceTransPitchPitch1` | 40 | 88 | 64 |
| `VoiceTransPitchPitch2` | 40 | 88 | 64 |
| `VoiceTransType` | 0 | 1 | 0 |

## GAME EFFECT (EQ, de-esser, surround) (101)

| Parameter | Min | Max | Default |
|---|---|---|---|
| `GameDeesserMAttack` | 57 | 283 | 237 |
| `GameDeesserMGain` | 0 | 180 | 37 |
| `GameDeesserMHiFreq` | 60 | 124 | 93 |
| `GameDeesserMLoFreq` | 4 | 96 | 36 |
| `GameDeesserMOnf` | 0 | 1 | 0 |
| `GameDeesserMRatio` | 0 | 120 | 10 |
| `GameDeesserMRelease` | 24 | 300 | 228 |
| `GameDeesserMThreshold` | 19 | 73 | 57 |
| `GameDeesserSAttack` | 57 | 283 | 237 |
| `GameDeesserSGain` | 0 | 180 | 37 |
| `GameDeesserSHiFreq` | 60 | 124 | 93 |
| `GameDeesserSLoFreq` | 4 | 96 | 36 |
| `GameDeesserSOnf` | 0 | 1 | 0 |
| `GameDeesserSRatio` | 0 | 120 | 10 |
| `GameDeesserSRelease` | 24 | 300 | 228 |
| `GameDeesserSThreshold` | 19 | 73 | 57 |
| `GameEffectName` | 0 | 255 | 0 |
| `GameEffectOnfa` | 0 | 1 | 0 |
| `GameEffectOnfb` | 0 | 1 | 0 |
| `GameEffectOnfc` | 0 | 1 | 0 |
| `GameEqMHiFreq` | 60 | 124 | 112 |
| `GameEqMHiGain` | 0 | 360 | 180 |
| `GameEqMHiOnf` | 0 | 1 | 1 |
| `GameEqMHiQual` | 0 | 60 | 12 |
| `GameEqMHiShape` | 0 | 1 | 1 |
| `GameEqMLoFreq` | 4 | 72 | 32 |
| `GameEqMLoGain` | 0 | 360 | 180 |
| `GameEqMLoOnf` | 0 | 1 | 1 |
| `GameEqMLoQual` | 0 | 60 | 12 |
| `GameEqMLoShape` | 0 | 1 | 1 |
| `GameEqMMhFreq` | 4 | 124 | 96 |
| `GameEqMMhGain` | 0 | 360 | 180 |
| `GameEqMMhOnf` | 0 | 1 | 1 |
| `GameEqMMhQual` | 0 | 60 | 12 |
| `GameEqMMlFreq` | 4 | 124 | 72 |
| `GameEqMMlGain` | 0 | 360 | 180 |
| `GameEqMMlOnf` | 0 | 1 | 1 |
| `GameEqMMlQual` | 0 | 60 | 12 |
| `GameEqMOnf` | 0 | 1 | 0 |
| `GameEqSHiFreq` | 60 | 124 | 112 |
| `GameEqSHiGain` | 0 | 360 | 180 |
| `GameEqSHiOnf` | 0 | 1 | 1 |
| `GameEqSHiQual` | 0 | 60 | 12 |
| `GameEqSHiShape` | 0 | 1 | 1 |
| `GameEqSLoFreq` | 4 | 72 | 32 |
| `GameEqSLoGain` | 0 | 360 | 180 |
| `GameEqSLoOnf` | 0 | 1 | 1 |
| `GameEqSLoQual` | 0 | 60 | 12 |
| `GameEqSLoShape` | 0 | 1 | 1 |
| `GameEqSMhFreq` | 4 | 124 | 96 |
| `GameEqSMhGain` | 0 | 360 | 180 |
| `GameEqSMhOnf` | 0 | 1 | 1 |
| `GameEqSMhQual` | 0 | 60 | 12 |
| `GameEqSMlFreq` | 4 | 124 | 72 |
| `GameEqSMlGain` | 0 | 360 | 180 |
| `GameEqSMlOnf` | 0 | 1 | 1 |
| `GameEqSMlQual` | 0 | 60 | 12 |
| `GameEqSOnf` | 0 | 1 | 0 |
| `GameEqTotalOnf` | 0 | 1 | 0 |
| `GameEqV210F` | 500 | 20000 | 16000 |
| `GameEqV210G` | 0 | 360 | 180 |
| `GameEqV210Onf` | 0 | 1 | 1 |
| `GameEqV210Q` | 0 | 60 | 42 |
| `GameEqV21F` | 20 | 1000 | 31 |
| `GameEqV21G` | 0 | 360 | 180 |
| `GameEqV21Onf` | 0 | 1 | 1 |
| `GameEqV21Q` | 0 | 60 | 42 |
| `GameEqV22F` | 20 | 20000 | 63 |
| `GameEqV22G` | 0 | 360 | 180 |
| `GameEqV22Onf` | 0 | 1 | 1 |
| `GameEqV22Q` | 0 | 60 | 42 |
| `GameEqV23F` | 20 | 20000 | 125 |
| `GameEqV23G` | 0 | 360 | 180 |
| `GameEqV23Onf` | 0 | 1 | 1 |
| `GameEqV23Q` | 0 | 60 | 42 |
| `GameEqV24F` | 20 | 20000 | 250 |
| `GameEqV24G` | 0 | 360 | 180 |
| `GameEqV24Onf` | 0 | 1 | 1 |
| `GameEqV24Q` | 0 | 60 | 42 |
| `GameEqV25F` | 20 | 20000 | 500 |
| `GameEqV25G` | 0 | 360 | 180 |
| `GameEqV25Onf` | 0 | 1 | 1 |
| `GameEqV25Q` | 0 | 60 | 42 |
| `GameEqV26F` | 20 | 20000 | 1000 |
| `GameEqV26G` | 0 | 360 | 180 |
| `GameEqV26Onf` | 0 | 1 | 1 |
| `GameEqV26Q` | 0 | 60 | 42 |
| `GameEqV27F` | 20 | 20000 | 2000 |
| `GameEqV27G` | 0 | 360 | 180 |
| `GameEqV27Onf` | 0 | 1 | 1 |
| `GameEqV27Q` | 0 | 60 | 42 |
| `GameEqV28F` | 20 | 20000 | 4000 |
| `GameEqV28G` | 0 | 360 | 180 |
| `GameEqV28Onf` | 0 | 1 | 1 |
| `GameEqV28Q` | 0 | 60 | 42 |
| `GameEqV29F` | 20 | 20000 | 8000 |
| `GameEqV29G` | 0 | 360 | 180 |
| `GameEqV29Onf` | 0 | 1 | 1 |
| `GameEqV29Q` | 0 | 60 | 42 |
| `GameFxOnf` | 0 | 1 | 0 |
| `GameFxType` | 0 | 100 | 0 |

## Monitor / headphone (27)

| Parameter | Min | Max | Default |
|---|---|---|---|
| `MonitorEqName` | 0 | 255 | 0 |
| `MonitorEqOnf` | 0 | 1 | 0 |
| `MonitorEqV21F` | 20 | 1000 | 100 |
| `MonitorEqV21G` | 0 | 360 | 180 |
| `MonitorEqV21Onf` | 0 | 1 | 1 |
| `MonitorEqV21Q` | 0 | 60 | 12 |
| `MonitorEqV21Shape` | 0 | 1 | 1 |
| `MonitorEqV22F` | 20 | 20000 | 500 |
| `MonitorEqV22G` | 0 | 360 | 180 |
| `MonitorEqV22Onf` | 0 | 1 | 1 |
| `MonitorEqV22Q` | 0 | 60 | 12 |
| `MonitorEqV22Shape` | 0 | 1 | 0 |
| `MonitorEqV23F` | 20 | 20000 | 2500 |
| `MonitorEqV23G` | 0 | 360 | 180 |
| `MonitorEqV23Onf` | 0 | 1 | 1 |
| `MonitorEqV23Q` | 0 | 60 | 12 |
| `MonitorEqV23Shape` | 0 | 1 | 0 |
| `MonitorEqV24F` | 500 | 20000 | 10000 |
| `MonitorEqV24G` | 0 | 360 | 180 |
| `MonitorEqV24Onf` | 0 | 1 | 1 |
| `MonitorEqV24Q` | 0 | 60 | 12 |
| `MonitorEqV24Shape` | 0 | 1 | 1 |
| `MonitorGameLevel` | 0 | 127 | 64 |
| `MonitorHpVolume` | 0 | 127 | 55 |
| `MonitorSelect` | 0 | 1 | 0 |
| `MonitorSpVolume` | 0 | 127 | 55 |
| `MonitorVoiceLevel` | 0 | 127 | 64 |

## Binaural (3D chat space) (41)

| Parameter | Min | Max | Default |
|---|---|---|---|
| `BinauralGOnf` | 0 | 1 | 0 |
| `BinauralGRevErDiffusion` | 0 | 10 | 10 |
| `BinauralGRevErFeedbackLevel` | 1 | 127 | 78 |
| `BinauralGRevErHpf` | 0 | 52 | 2 |
| `BinauralGRevErInitDelay` | 0 | 127 | 3 |
| `BinauralGRevErLpf` | 34 | 60 | 50 |
| `BinauralGRevErRoomsize` | 0 | 63 | 15 |
| `BinauralGRevErType` | 0 | 5 | 0 |
| `BinauralGRevLevel` | 0 | 103 | 103 |
| `BinauralGRevOnf` | 0 | 1 | 0 |
| `BinauralGRevRevDensity` | 0 | 4 | 4 |
| `BinauralGRevRevDiffusion` | 0 | 10 | 10 |
| `BinauralGRevRevErSel` | 0 | 1 | 0 |
| `BinauralGRevRevHighRatio` | 1 | 10 | 8 |
| `BinauralGRevRevHpf` | 0 | 52 | 0 |
| `BinauralGRevRevInitDelay` | 0 | 63 | 16 |
| `BinauralGRevRevLpf` | 34 | 60 | 53 |
| `BinauralGRevRevTime` | 0 | 57 | 25 |
| `BinauralGRevRevType` | 0 | 4 | 0 |
| `BinauralGRevType` | 0 | 4 | 0 |
| `BinauralVOnf` | 0 | 1 | 0 |
| `BinauralVPosition` | 0 | 16 | 3 |
| `BinauralVRevErDiffusion` | 0 | 10 | 10 |
| `BinauralVRevErFeedbackLevel` | 1 | 127 | 78 |
| `BinauralVRevErHpf` | 0 | 52 | 2 |
| `BinauralVRevErInitDelay` | 0 | 127 | 3 |
| `BinauralVRevErLpf` | 34 | 60 | 50 |
| `BinauralVRevErRoomsize` | 0 | 63 | 15 |
| `BinauralVRevErType` | 0 | 5 | 0 |
| `BinauralVRevLevel` | 0 | 103 | 103 |
| `BinauralVRevOnf` | 0 | 1 | 0 |
| `BinauralVRevRevDensity` | 0 | 4 | 4 |
| `BinauralVRevRevDiffusion` | 0 | 10 | 10 |
| `BinauralVRevRevErSel` | 0 | 1 | 0 |
| `BinauralVRevRevHighRatio` | 1 | 10 | 8 |
| `BinauralVRevRevHpf` | 0 | 52 | 0 |
| `BinauralVRevRevInitDelay` | 0 | 63 | 16 |
| `BinauralVRevRevLpf` | 34 | 60 | 53 |
| `BinauralVRevRevTime` | 0 | 57 | 25 |
| `BinauralVRevRevType` | 0 | 4 | 0 |
| `BinauralVRevType` | 0 | 4 | 0 |

## Streaming output mixer (USB, HDMI, PC) (22)

| Parameter | Min | Max | Default |
|---|---|---|---|
| `Hdmi14KMode` | 0 | 1 | 0 |
| `Hdmi24KMode` | 0 | 1 | 0 |
| `HdmiHdcpVersion1` | 0 | 1 | 0 |
| `HdmiHdcpVersion2` | 0 | 1 | 0 |
| `HdmiMute` | 0 | 1 | 0 |
| `HdmiSelect` | 0 | 1 | 0 |
| `StreamingGameFx` | 0 | 1 | 0 |
| `StreamingGameLevel` | 0 | 127 | 103 |
| `StreamingGameLink` | 0 | 1 | 0 |
| `StreamingGameMute` | 0 | 1 | 0 |
| `StreamingHdmiOutOnf` | 0 | 1 | 1 |
| `StreamingMicLevel` | 0 | 127 | 0 |
| `StreamingMicLink` | 0 | 1 | 0 |
| `StreamingMicMute` | 0 | 1 | 0 |
| `StreamingMonitor` | 0 | 1 | 0 |
| `StreamingUsbOutOnf` | 0 | 1 | 0 |
| `StreamingVoiceFx` | 0 | 1 | 0 |
| `StreamingVoiceLevel` | 0 | 127 | 0 |
| `StreamingVoiceLink` | 0 | 1 | 0 |
| `StreamingVoiceMute` | 0 | 1 | 0 |
| `ToPcMuteStreaming` | 0 | 1 | 0 |
| `ToPcMuteVoice` | 0 | 1 | 0 |

## Presets and current selection (6)

| Parameter | Min | Max | Default |
|---|---|---|---|
| `CurrentGameEffectButton` | 0 | 2 | 0 |
| `L11` | 1 | 2147676892 | 2147676896 |
| `PresetVoiceName` | 0 | 255 | 0 |
| `PresetVoiceOnf1` | 0 | 1 | 0 |
| `PresetVoiceOnf2` | 0 | 1 | 0 |
| `R10` | 1 | 2147676972 | 2147676976 |

## Other controls (11)

| Parameter | Min | Max | Default |
|---|---|---|---|
| `DirtyFlagStreaming` | 0 | 1 | 0 |
| `LedBrightness` | 0 | 1 | 1 |
| `game_eq_v2_3_G` | 1 | 2147670296 | 2147671632 |
| `game_eq_v2_7_F` | 1 | 2147669800 | 2147671488 |
| `game_eq_v2_8_ONF` | 1 | 2147669448 | 2147676088 |
| `mic_eq_v2_2_G` | 1 | 2147673136 | 2147672200 |
| `mic_eq_v2_2_Q` | 1 | 2147674744 | 2147671360 |
| `mic_eq_v2_3_ONF` | 1 | 2147675920 | 2147675144 |
| `mic_eq_v2_4_ONF` | 1 | 2147672768 | 2147670456 |
| `monitor_eq_v2_2_SHAPE` | 1 | 2147671148 | 2147672880 |
| `monitor_eq_v2_3_ONF` | 1 | 2147674648 | 2147672720 |

## Legacy snake_case names (52)

A second naming layer present in the same library.  Several
of these are read-only status entries rather than settings,
for example `sys_samplingrate`, `inpt_is_anlg`,
`outpt_is_anlg`, `sys_hs_loopback`, `meter_mic_output`.

| Parameter | Min | Max | Default |
|---|---|---|---|
| `bina_g_rev_rev_den` | 1 | 2147672864 | 2147673600 |
| `bina_v_rev_er_ini` | 1 | 2147671800 | 2147669424 |
| `bina_v_rev_er_room` | 1 | 2147670720 | 2147673040 |
| `bina_v_rev_on` | 1 | 2147670408 | 2147668840 |
| `bina_v_rev_rev_hi_r` | 1 | 2147673152 | 2147669048 |
| `dirty_streming` | 1 | 2147675776 | 2147672368 |
| `gam_dee_s_lo` | 1 | 2147672912 | 2147669768 |
| `gam_eq_s_hi_g` | 1 | 2147669128 | 2147670216 |
| `gam_eq_s_lo_q` | 1 | 2147671192 | 2147671520 |
| `gam_eq_s_mh_g` | 1 | 2147674200 | 2147671216 |
| `gam_eq_s_ml_g` | 1 | 2147675032 | 2147672016 |
| `gam_eq_t_hi_f` | 1 | 2147671296 | 2147670640 |
| `gam_eq_t_mh_q` | 1 | 2147672496 | 2147669832 |
| `gam_eq_t_ml_q` | 1 | 2147669552 | 2147672736 |
| `inpt_is_anlg` | 1 | 2147669272 | 2147672832 |
| `meter_mic_output` | 1 | 2147672432 | 2147674064 |
| `meter_pc_in` | 1 | 2147671848 | 2147670736 |
| `mic_comp_knee` | 1 | 2147668904 | 2147673760 |
| `mic_eq_hi_sh` | 1 | 2147673504 | 2147672416 |
| `mic_eq_lo_f` | 1 | 2147676184 | 2147672344 |
| `mic_func2` | 1 | 2147675272 | 2147673312 |
| `mic_limt_outgain` | 1 | 2147672800 | 2147670056 |
| `mic_mute_onf` | 1 | 2147670928 | 2147670240 |
| `mon_eq_lo_q` | 1 | 2147674136 | 2147672248 |
| `mon_eq_lo_sh` | 1 | 2147675304 | 2147674408 |
| `mon_eq_mh_f` | 1 | 2147674120 | 2147672752 |
| `mon_eq_ml_f` | 1 | 2147669496 | 2147670832 |
| `mon_eq_ml_q` | 1 | 2147669208 | 2147671280 |
| `mon_select` | 1 | 2147670592 | 2147674960 |
| `outpt_is_anlg` | 1 | 2147672128 | 2147669112 |
| `pre_vic_name` | 1 | 2147671752 | 2147669912 |
| `pre_vic_on1` | 1 | 2147670784 | 2147671056 |
| `pre_vic_on2` | 1 | 2147670280 | 2147671944 |
| `se_level` | 1 | 2147671456 | 2147672576 |
| `spc_sys_cnt_outprt` | 1 | 2147672464 | 2147672544 |
| `str_inprt_name` | 1 | 2147671776 | 2147675944 |
| `strm_mic_mute` | 1 | 2147670864 | 2147674424 |
| `strm_out_onf` | 1 | 2147673776 | 2147673584 |
| `strm_voice_fx` | 1 | 2147672704 | 2147675240 |
| `strm_voice_mute` | 1 | 2147675448 | 2147675512 |
| `sys_hs_loopback` | 1 | 2147675704 | 2147670040 |
| `sys_is_mtrx_crosstie` | 1 | 2147675080 | 2147674360 |
| `sys_samplingrate` | 1 | 2147675824 | 2147673840 |
| `vic_fx_rdo_cut` | 1 | 2147674512 | 2147671208 |
| `vic_fx_rdo_lpf` | 1 | 2147670184 | 2147673536 |
| `vic_rev_wet` | 1 | 2147674792 | 2147672080 |
| `vic_tc_hi_f` | 1 | 2147668768 | 2147670200 |
| `vic_tc_hi_sh` | 1 | 2147669656 | 2147669896 |
| `vic_tc_lo_f` | 1 | 2147670960 | 2147674728 |
| `vic_tc_md_g` | 1 | 2147674760 | 2147674776 |
| `voice_harm_level` | 1 | 2147674984 | 2147675352 |
| `voice_pitch_level` | 1 | 2147674856 | 2147669472 |

