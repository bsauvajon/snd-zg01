#!/bin/bash
# Test ZG01 driver on localhost (after disabling Secure Boot)

echo "=== ZG01 Localhost Test ==="
echo ""

# Load modules
echo "1. Loading driver modules..."
cd /home/brice/repos/snd-zg01
sudo ./scripts/load_modules.sh
if [ $? -ne 0 ]; then
    echo "ERROR: Failed to load modules. Did you disable Secure Boot?"
    exit 1
fi

echo ""
echo "2. Checking sound cards..."
cat /proc/asound/cards | grep -A 1 zg01
aplay -l | grep zg01

echo ""
echo "3. Testing audio playback and recording..."
echo "   Starting external microphone recording on hw:2,0..."

# Record 6 seconds with external mic while playing test tone
(arecord -D hw:2,0 -f S24_3LE -r 48000 -c 2 -t wav -d 6 test_localhost.wav 2>/dev/null &)
sleep 1

echo "   Playing 440Hz sine wave on ZG01..."
speaker-test -D hw:zg01game -c 2 -r 48000 -F S32_LE -t sine -f 440 -l 1 >/dev/null 2>&1

sleep 5

echo ""
echo "4. Analyzing audio quality..."
if [ -f test_localhost.wav ]; then
    ffmpeg -i test_localhost.wav -af "volumedetect,adeclick" -f null - 2>&1 | grep -E "(mean_volume|max_volume|Parsed_adeclick|detected)"
    
    echo ""
    echo "=== Results Summary ==="
    echo "Recording: test_localhost.wav"
    ls -lh test_localhost.wav
    echo ""
    echo "Compare with VM results:"
    echo "  VM (original): ~2.1% clicks"
    echo "  Localhost:     (see above)"
    echo ""
    echo "If clicks are much lower on localhost, VM overhead is the issue."
    echo "If clicks are similar, it's a driver/buffering issue."
else
    echo "ERROR: Recording file not created"
    exit 1
fi
