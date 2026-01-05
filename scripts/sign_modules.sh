#!/bin/bash
# Sign kernel modules for Secure Boot

# Generate key pair if not exists
if [ ! -f /var/lib/shim-signed/mok/MOK.priv ]; then
    echo "Generating MOK keys..."
    sudo mkdir -p /var/lib/shim-signed/mok
    sudo openssl req -new -x509 -newkey rsa:2048 -keyout /var/lib/shim-signed/mok/MOK.priv -outform DER -out /var/lib/shim-signed/mok/MOK.der -days 36500 -subj "/CN=ZG01 Driver/" -nodes
fi

# Sign all modules
for module in src/zg01_pcm.ko src/zg01_control.ko src/zg01_usb_discovery.ko src/zg01_usb.ko; do
    echo "Signing $module..."
    sudo /usr/src/linux-headers-$(uname -r)/scripts/sign-file sha256 \
        /var/lib/shim-signed/mok/MOK.priv \
        /var/lib/shim-signed/mok/MOK.der \
        $module
done

echo ""
echo "Modules signed! Now enroll the key with:"
echo "  sudo mokutil --import /var/lib/shim-signed/mok/MOK.der"
echo "Then reboot and follow the MOK Manager prompts to enroll the key."
