#include <linux/usb.h>
#include <linux/module.h>
#include "zg01.h"

/* USB Hardware Configuration Discovery for Yamaha ZG01 */

struct zg01_endpoint_info {
    u8 address;
    u8 attributes;
    u16 max_packet_size;
    u8 interval;
    bool is_audio;
    const char *type_name;
};

struct zg01_interface_info {
    u8 interface_num;
    u8 alt_setting;
    u8 num_endpoints;
    struct zg01_endpoint_info endpoints[16];
};

static const char* get_endpoint_type_name(u8 attributes)
{
    switch (attributes & USB_ENDPOINT_XFERTYPE_MASK) {
        case USB_ENDPOINT_XFER_CONTROL:
            return "Control";
        case USB_ENDPOINT_XFER_ISOC:
            return "Isochronous";
        case USB_ENDPOINT_XFER_BULK:
            return "Bulk";
        case USB_ENDPOINT_XFER_INT:
            return "Interrupt";
        default:
            return "Unknown";
    }
}

static const char* get_endpoint_direction(u8 address)
{
    return (address & USB_DIR_IN) ? "IN" : "OUT";
}

static bool is_audio_endpoint(struct usb_endpoint_descriptor *ep_desc, 
                              struct usb_interface_descriptor *intf_desc)
{
    /* Check if this looks like an audio endpoint */
    u8 ep_type = ep_desc->bmAttributes & USB_ENDPOINT_XFERTYPE_MASK;
    
    /* Audio endpoints are typically isochronous or bulk */
    if (ep_type == USB_ENDPOINT_XFER_ISOC || ep_type == USB_ENDPOINT_XFER_BULK) {
        /* Check packet size - audio endpoints usually have larger packets */
        u16 max_packet = le16_to_cpu(ep_desc->wMaxPacketSize);
        if (max_packet >= 64) { /* Reasonable audio packet size */
            return true;
        }
    }
    
    return false;
}

static void zg01_discover_all_alt_settings(struct usb_interface *interface)
{
    int alt_idx;
    
    pr_info("zg01_discovery: Discovering all alternate settings for interface %d\n",
            interface->cur_altsetting->desc.bInterfaceNumber);
    
    for (alt_idx = 0; alt_idx < interface->num_altsetting; alt_idx++) {
        struct usb_host_interface *altsetting = &interface->altsetting[alt_idx];
        struct zg01_interface_info info;
        int ep_idx;
        
        info.interface_num = altsetting->desc.bInterfaceNumber;
        info.alt_setting = altsetting->desc.bAlternateSetting;
        info.num_endpoints = altsetting->desc.bNumEndpoints;
        
        pr_info("zg01_discovery: === Alt Setting %d ===\n", info.alt_setting);
        pr_info("zg01_discovery:   Endpoints: %d\n", info.num_endpoints);
        pr_info("zg01_discovery:   Class: 0x%02x, SubClass: 0x%02x, Protocol: 0x%02x\n",
                altsetting->desc.bInterfaceClass,
                altsetting->desc.bInterfaceSubClass,
                altsetting->desc.bInterfaceProtocol);
        
        for (ep_idx = 0; ep_idx < info.num_endpoints && ep_idx < 16; ep_idx++) {
            struct usb_endpoint_descriptor *ep_desc = &altsetting->endpoint[ep_idx].desc;
            struct zg01_endpoint_info *ep_info = &info.endpoints[ep_idx];
            
            ep_info->address = ep_desc->bEndpointAddress;
            ep_info->attributes = ep_desc->bmAttributes;
            ep_info->max_packet_size = le16_to_cpu(ep_desc->wMaxPacketSize);
            ep_info->interval = ep_desc->bInterval;
            ep_info->is_audio = is_audio_endpoint(ep_desc, &altsetting->desc);
            ep_info->type_name = get_endpoint_type_name(ep_info->attributes);
            
            pr_info("zg01_discovery:     EP 0x%02x: %s %s, MaxPacket=%d, Interval=%d%s\n",
                    ep_info->address,
                    get_endpoint_direction(ep_info->address),
                    ep_info->type_name,
                    ep_info->max_packet_size,
                    ep_info->interval,
                    ep_info->is_audio ? " [AUDIO]" : "");
        }
    }
}

int zg01_discover_usb_config(struct zg01_dev *dev)
{
    struct usb_device *udev = dev->udev;
    struct usb_interface *interface = dev->interface;
    int i;
    
    if (!udev || !interface) {
        pr_err("zg01_discovery: Invalid device or interface\n");
        return -EINVAL;
    }
    
    pr_info("zg01_discovery: ========================================\n");
    pr_info("zg01_discovery: USB Hardware Discovery for Yamaha ZG01\n");
    pr_info("zg01_discovery: ========================================\n");
    
    /* Device-level information */
    pr_info("zg01_discovery: Device: %04x:%04x (USB %d.%d)\n",
            le16_to_cpu(udev->descriptor.idVendor),
            le16_to_cpu(udev->descriptor.idProduct),
            (udev->descriptor.bcdUSB >> 8) & 0xff,
            udev->descriptor.bcdUSB & 0xff);
    
    pr_info("zg01_discovery: Speed: %s\n", 
            udev->speed == USB_SPEED_HIGH ? "High Speed (480 Mbps)" :
            udev->speed == USB_SPEED_FULL ? "Full Speed (12 Mbps)" :
            udev->speed == USB_SPEED_LOW ? "Low Speed (1.5 Mbps)" :
            udev->speed == USB_SPEED_SUPER ? "Super Speed (5 Gbps)" : "Unknown");
    
    /* Current configuration */
    if (udev->actconfig) {
        pr_info("zg01_discovery: Current Configuration: %d (%d interfaces)\n",
                udev->actconfig->desc.bConfigurationValue,
                udev->actconfig->desc.bNumInterfaces);
    }
    
    /* Current interface discovery */
    pr_info("zg01_discovery: ========================================\n");
    pr_info("zg01_discovery: Current Interface Analysis\n");
    pr_info("zg01_discovery: ========================================\n");
    
    zg01_discover_all_alt_settings(interface);
    
    /* Try to find the best audio endpoints */
    pr_info("zg01_discovery: ========================================\n");
    pr_info("zg01_discovery: Audio Endpoint Recommendations\n");
    pr_info("zg01_discovery: ========================================\n");
    
    /* Scan all alternate settings for audio endpoints */
    for (i = 0; i < interface->num_altsetting; i++) {
        struct usb_host_interface *alt = &interface->altsetting[i];
        int ep_idx;
        
        for (ep_idx = 0; ep_idx < alt->desc.bNumEndpoints; ep_idx++) {
            struct usb_endpoint_descriptor *ep_desc = &alt->endpoint[ep_idx].desc;
            u16 max_packet = le16_to_cpu(ep_desc->wMaxPacketSize);
            
            if (is_audio_endpoint(ep_desc, &alt->desc)) {
                pr_info("zg01_discovery: RECOMMENDED: Interface %d, Alt %d, EP 0x%02x (%s %s, %d bytes)\n",
                        alt->desc.bInterfaceNumber,
                        alt->desc.bAlternateSetting,
                        ep_desc->bEndpointAddress,
                        get_endpoint_direction(ep_desc->bEndpointAddress),
                        get_endpoint_type_name(ep_desc->bmAttributes),
                        max_packet);
                        
                /* Check if this matches our pcap analysis */
                if ((ep_desc->bEndpointAddress & 0x0F) == 1) {
                    if (max_packet == 512) {
                        pr_info("zg01_discovery:   -> MATCHES Voice channel from pcap (512 bytes)\n");
                    } else if (max_packet >= 8192) {
                        pr_info("zg01_discovery:   -> MATCHES Game channel from pcap (8192 bytes)\n");
                    }
                }
            }
        }
    }
    
    pr_info("zg01_discovery: ========================================\n");
    pr_info("zg01_discovery: Discovery Complete\n");
    pr_info("zg01_discovery: ========================================\n");
    
    return 0;
}

/* Function to find and configure the best audio endpoint */
int zg01_find_audio_endpoint(struct zg01_dev *dev, u8 *endpoint_addr, u8 *alt_setting)
{
    struct usb_interface *interface = dev->interface;
    int i, ep_idx;
    u8 best_endpoint = 0;
    u8 best_alt = 0;
    u16 best_packet_size = 0;
    
    /* Scan all alternate settings for the best audio endpoint */
    for (i = 0; i < interface->num_altsetting; i++) {
        struct usb_host_interface *alt = &interface->altsetting[i];
        
        for (ep_idx = 0; ep_idx < alt->desc.bNumEndpoints; ep_idx++) {
            struct usb_endpoint_descriptor *ep_desc = &alt->endpoint[ep_idx].desc;
            u16 max_packet = le16_to_cpu(ep_desc->wMaxPacketSize);
            
            if (is_audio_endpoint(ep_desc, &alt->desc)) {
                /* Prefer OUT endpoints for playback */
                if (!(ep_desc->bEndpointAddress & USB_DIR_IN)) {
                    if (max_packet > best_packet_size) {
                        best_endpoint = ep_desc->bEndpointAddress;
                        best_alt = alt->desc.bAlternateSetting;
                        best_packet_size = max_packet;
                    }
                }
            }
        }
    }
    
    if (best_endpoint) {
        *endpoint_addr = best_endpoint;
        *alt_setting = best_alt;
        pr_info("zg01_discovery: Selected endpoint 0x%02x (alt %d, %d bytes) for audio\n",
                best_endpoint, best_alt, best_packet_size);
        return 0;
    }
    
    pr_err("zg01_discovery: No suitable audio endpoint found\n");
    return -ENODEV;
}

EXPORT_SYMBOL(zg01_discover_usb_config);
EXPORT_SYMBOL(zg01_find_audio_endpoint);

MODULE_AUTHOR("ZG01 Driver Team");
MODULE_DESCRIPTION("USB Hardware Discovery for Yamaha ZG01");
MODULE_LICENSE("GPL");
MODULE_VERSION("1.0");