#!/usr/bin/env bash
# L-Helper hardware probe for the Linux port (docs/linux-port.md, phase 0).
#
#   bash probe.sh
#
# Read-only: it reads /sys, /proc and a few tools' output and writes one archive to your home folder.
# It changes no settings, loads no drivers and writes nothing to /sys or to devices.
# It asks for sudo once, for the firmware tables, WMI descriptions and kernel messages; without sudo it
# still runs and leaves those out. Serial numbers, UUIDs, MAC addresses and the MSDM table (Windows key)
# are never collected.
set -u -o pipefail
shopt -s nullglob

PROBE_VERSION=1
SCRIPT=$(readlink -f "${BASH_SOURCE[0]}")

have() { command -v "$1" >/dev/null 2>&1; }
T=; have timeout && T="timeout 20"

# Desktop details live on the user's session bus, which root can't see: collect them before sudo
desktop_info() {
    echo "session type: ${XDG_SESSION_TYPE:-?}  desktop: ${XDG_CURRENT_DESKTOP:-?}  session: ${DESKTOP_SESSION:-?}  wayland: ${WAYLAND_DISPLAY:+yes}"
    have gnome-shell && gnome-shell --version
    have plasmashell && plasmashell --version
    if have gnome-extensions; then
        echo "GNOME extensions enabled:"
        gnome-extensions list --enabled 2>/dev/null | sed 's/^/  /'
    fi
    if have busctl; then
        if busctl --user list 2>/dev/null | grep -q StatusNotifierWatcher; then echo "tray host (StatusNotifierWatcher): present"
        else echo "tray host (StatusNotifierWatcher): missing"; fi
    fi
    for t in xrandr kscreen-doctor gnome-monitor-config wlr-randr powerprofilesctl mangohud nvidia-smi fwupdmgr; do
        printf '%-22s %s\n' "$t" "$(command -v "$t" || echo -)"
    done
    if have powerprofilesctl; then
        echo "power-profiles-daemon:"
        $T powerprofilesctl list 2>&1 | sed 's/^/  /'
    fi
}

if [ "$(id -u)" -ne 0 ] && [ -z "${LHP_STAGE:-}" ]; then
    desk=$(mktemp)
    desktop_info > "$desk" 2>&1
    if have sudo; then
        echo "L-Helper probe: asking for sudo to read firmware tables and kernel messages (read-only)."
        if sudo -v; then
            exec sudo env LHP_STAGE=root LHP_USER="$(id -un)" LHP_HOME="$HOME" LHP_DESKTOP_FILE="$desk" bash "$SCRIPT"
        fi
        echo "No sudo: continuing without it, so some sections will be empty."
    fi
    LHP_DESKTOP_FILE=$desk
fi

USER_NAME=${LHP_USER:-${SUDO_USER:-$(id -un)}}
USER_HOME=${LHP_HOME:-$(getent passwd "$USER_NAME" 2>/dev/null | cut -d: -f6)}
USER_HOME=${USER_HOME:-$HOME}

model=$(tr -cs 'A-Za-z0-9' '-' 2>/dev/null < /sys/class/dmi/id/product_name |sed 's/^-*//; s/-*$//')
work=$(mktemp -d)
out="$work/lhelper-probe-${model:-unknown}-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$out/acpi" "$out/wmi" "$out/hid"
R="$out/report.txt"
: > "$R"

section() { printf '\n===== %s =====\n' "$1" >> "$R"; }

# "path = value" for each readable file (multi-line values indented below the path)
val() {
    local f v
    for f in "$@"; do
        [ -f "$f" ] || continue
        if v=$($T head -c 4000 "$f" 2>/dev/null | tr '\0' ' '); then
            case $v in
                *$'\n'*) printf '%s:\n%s\n' "$f" "$(printf '%s\n' "$v" | sed 's/^/    /')" ;;
                *) printf '%s = %s\n' "$f" "$v" ;;
            esac
        else
            printf '%s = (unreadable)\n' "$f"
        fi
    done >> "$R"
}

# Every regular file directly in a directory
val_dir() {
    local d=$1 f
    for f in "$d"/*; do [ -f "$f" ] && val "$f"; done
}

run() {
    echo "\$ $*" >> "$R"
    if have "$1"; then $T "$@" >> "$R" 2>&1; else echo "  ($1 not installed)" >> "$R"; fi
}

echo "Collecting... (read-only)"

# ---------------- system ----------------
section "Probe"
{
    echo "probe version: $PROBE_VERSION"
    echo "date: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "running as: $(id -un)$([ "$(id -u)" -eq 0 ] && echo ' (root)')"
} >> "$R"

section "System"
{
    grep -E '^(NAME|VERSION|ID|ID_LIKE|VERSION_ID|PRETTY_NAME)=' /etc/os-release 2>/dev/null
    echo "kernel: $(uname -r)  $(uname -v)  $(uname -m)"
    grep -m1 'model name' /proc/cpuinfo
    echo "memory: $(awk '/MemTotal/ {printf "%.1f GiB", $2/1048576}' /proc/meminfo)"
} >> "$R"
for f in sys_vendor product_name product_version product_family product_sku board_vendor board_name board_version \
         bios_vendor bios_version bios_date bios_release ec_firmware_release chassis_type; do
    val "/sys/class/dmi/id/$f"
done
if have mokutil; then
    run mokutil --sb-state
else
    sb=$(od -An -t u1 -j4 -N1 /sys/firmware/efi/efivars/SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c 2>/dev/null | tr -d ' ')
    echo "Secure Boot (efivar): ${sb:-unknown} (1 = on)" >> "$R"
fi
val /sys/kernel/security/lockdown

section "Desktop (collected as $USER_NAME)"
if [ -n "${LHP_DESKTOP_FILE:-}" ] && [ -r "$LHP_DESKTOP_FILE" ]; then
    cat "$LHP_DESKTOP_FILE" >> "$R"
    rm -f "$LHP_DESKTOP_FILE"
else
    echo "(not collected: run the probe as your normal user, not with sudo, to include this)" >> "$R"
fi

# ---------------- kernel drivers ----------------
section "Kernel modules"
if have lsmod; then
    lsmod | grep -iE '^Module|lenovo|ideapad|legion|thinkpad|wmi|platform_profile|firmware_attributes|nvidia|nouveau|amdgpu|i915|k10temp|coretemp|zenpower|hid' >> "$R"
fi
echo >> "$R"
for m in lenovo_wmi_gamezone lenovo_wmi_other lenovo_wmi_capdata01 lenovo_wmi_capdata lenovo_wmi_events lenovo_wmi_helpers \
         lenovo_wmi_hotkey_utilities lenovo_wmi_camera ideapad_laptop legion_laptop thinkpad_acpi platform_profile \
         firmware_attributes_class nvidia nouveau; do
    printf '%-28s %s\n' "$m" "$(modinfo -F filename "$m" 2>/dev/null || echo 'not available')" >> "$R"
done
cfg=/boot/config-$(uname -r)
pat='CONFIG_(LENOVO|IDEAPAD|THINKPAD|LEGION|ACPI_PLATFORM_PROFILE|FW_ATTR|ACPI_WMI|WMI_BMOF|HID_LENOVO)'
if [ -r "$cfg" ]; then echo "$cfg:" >> "$R"; grep -E "$pat" "$cfg" >> "$R"
elif [ -r /proc/config.gz ] && have zgrep; then echo "/proc/config.gz:" >> "$R"; zgrep -E "$pat" /proc/config.gz >> "$R"
fi
have dkms && run dkms status
echo >> "$R"
echo "WMI drivers: $(ls /sys/bus/wmi/drivers 2>/dev/null | tr '\n' ' ')" >> "$R"
echo "Platform drivers: $(ls /sys/bus/platform/drivers 2>/dev/null | grep -iE 'lenovo|ideapad|legion|thinkpad' | tr '\n' ' ')" >> "$R"

# ---------------- firmware interfaces ----------------
section "WMI devices (/sys/bus/wmi/devices)"
for d in /sys/bus/wmi/devices/*; do
    name=$(basename "$d")
    drv=-; [ -e "$d/driver" ] && drv=$(basename "$(readlink -f "$d/driver")")
    line="$name driver=$drv"
    for a in instance_count expensive object_id notify_id setable; do
        [ -f "$d/$a" ] && line="$line $a=$(cat "$d/$a" 2>/dev/null)"
    done
    # Binary MOF: the class names and method signatures behind each GUID (root only)
    if [ -f "$d/bmof" ] && cat "$d/bmof" > "$out/wmi/$name.bmof" 2>/dev/null; then line="$line (bmof saved)"
    else rm -f "$out/wmi/$name.bmof"; fi
    echo "$line" >> "$R"
done

section "ACPI tables"
for t in /sys/firmware/acpi/tables/*; do
    [ -f "$t" ] || continue
    n=$(basename "$t")
    case $n in
        DSDT|SSDT*) if cat "$t" > "$out/acpi/$n" 2>/dev/null; then echo "$n saved" >> "$R"; else rm -f "$out/acpi/$n"; echo "$n (unreadable)" >> "$R"; fi ;;
        *) echo "$n" >> "$R" ;;
    esac
done

section "Platform profile"
val /sys/firmware/acpi/platform_profile /sys/firmware/acpi/platform_profile_choices
for d in /sys/class/platform-profile/*; do val "$d/name" "$d/profile" "$d/choices"; done

section "Firmware attributes (/sys/class/firmware-attributes)"
for c in /sys/class/firmware-attributes/*; do
    echo "[$(basename "$c")]" >> "$R"
    for a in "$c"/attributes/*/; do val_dir "${a%/}"; done
    val "$c/attributes/pending_reboot"
done

section "Lenovo platform devices"
ls /sys/bus/platform/devices 2>/dev/null | grep -iE 'VPC|PNP0C09|lenovo|ideapad|legion' >> "$R"
for d in /sys/bus/platform/drivers/ideapad_acpi/*/ /sys/bus/platform/drivers/legion/*/ /sys/bus/platform/drivers/*lenovo*/*/; do
    [ "$(basename "$d")" = module ] && continue
    echo "[$(readlink -f "$d")]" >> "$R"
    val_dir "${d%/}"
done

# ---------------- power and sensors ----------------
section "Power supply"
for p in /sys/class/power_supply/*; do
    echo "[$(basename "$p")]" >> "$R"
    for f in type status online capacity capacity_level energy_now energy_full energy_full_design charge_now charge_full \
             power_now current_now voltage_now charge_control_start_threshold charge_control_end_threshold charge_types \
             charge_type charge_behaviour manufacturer model_name technology cycle_count; do
        val "$p/$f"
    done
done

section "CPU power"
val /sys/devices/system/cpu/amd_pstate/status /sys/devices/system/cpu/intel_pstate/status \
    /sys/devices/system/cpu/intel_pstate/no_turbo /sys/devices/system/cpu/cpufreq/boost \
    /sys/devices/system/cpu/cpu0/cpufreq/{scaling_driver,scaling_governor,energy_performance_preference,energy_performance_available_preferences,cpuinfo_max_freq,scaling_max_freq}
for z in /sys/class/powercap/*; do
    val "$z/name" "$z/enabled" "$z/energy_uj" "$z"/constraint_*_name "$z"/constraint_*_power_limit_uw "$z"/constraint_*_time_window_us
done

section "Sensors (hwmon)"
for h in /sys/class/hwmon/hwmon*; do
    dev=-; [ -e "$h/device" ] && dev=$(readlink -f "$h/device")
    echo "[$(basename "$h")] $(cat "$h/name" 2>/dev/null)  device=$dev" >> "$R"
    val "$h"/temp*_label "$h"/temp*_input "$h"/temp*_crit "$h"/fan*_label "$h"/fan*_input "$h"/fan*_min "$h"/fan*_max \
        "$h"/fan*_target "$h"/pwm* "$h"/power*_label "$h"/power*_input "$h"/power*_average "$h"/power*_cap* \
        "$h"/in*_label "$h"/in*_input "$h"/freq*_label "$h"/freq*_input
done

# ---------------- GPU and displays ----------------
section "GPU"
if have lspci; then lspci -nnk 2>/dev/null | grep -EA3 'VGA|3D controller|Display controller' >> "$R"; else echo "(lspci not installed)" >> "$R"; fi
for c in /sys/class/drm/card[0-9]; do
    drv=-; [ -e "$c/device/driver" ] && drv=$(basename "$(readlink -f "$c/device/driver")")
    echo "[$(basename "$c")] driver=$drv" >> "$R"
    val "$c/device/vendor" "$c/device/device" "$c/device/boot_vga" "$c/device/power_state" \
        "$c/device/power/control" "$c/device/power/runtime_status" "$c/device/power_dpm_force_performance_level"
done
val /proc/driver/nvidia/version
if have nvidia-smi; then
    echo "\$ nvidia-smi -q (serials and UUIDs removed)" >> "$R"
    $T nvidia-smi -q 2>&1 | grep -viE 'serial|uuid' >> "$R"
fi

section "Displays (DRM connectors)"
for c in /sys/class/drm/card*-*; do
    echo "[$(basename "$c")] status=$(cat "$c/status" 2>/dev/null) enabled=$(cat "$c/enabled" 2>/dev/null)" >> "$R"
    [ -f "$c/modes" ] && sort -u "$c/modes" 2>/dev/null | head -20 | sed 's/^/    /' >> "$R"
done
for b in /sys/class/backlight/*; do val "$b/type" "$b/max_brightness" "$b/brightness"; done

# ---------------- keyboard and keys ----------------
section "HID devices (hidraw)"
for h in /sys/class/hidraw/hidraw*; do
    n=$(basename "$h")
    ids=$(grep -E '^HID_(ID|NAME|PHYS)=' "$h/device/uevent" 2>/dev/null | tr '\n' ' ')
    iface=$(cat "$h/device/../bInterfaceNumber" 2>/dev/null)
    drv=-; [ -e "$h/device/driver" ] && drv=$(basename "$(readlink -f "$h/device/driver")")
    echo "$n interface=${iface:--} driver=$drv $ids" >> "$R"
    # Report descriptor: report IDs and feature report sizes for the lighting protocol
    cat "$h/device/report_descriptor" > "$out/hid/$n.rdesc" 2>/dev/null || rm -f "$out/hid/$n.rdesc"
done
have lsusb && run lsusb

section "Input devices"
grep -vE '^U: Uniq=' /proc/bus/input/devices >> "$R" 2>/dev/null

section "LEDs"
for l in /sys/class/leds/*; do val "$l/brightness" "$l/max_brightness"; done

# ---------------- services and kernel log ----------------
section "Power and hardware services"
if have systemctl; then
    for s in power-profiles-daemon tuned tuned-ppd tlp auto-cpufreq thermald system76-power supergfxd asusd \
             nvidia-powerd lenovo-legion legiond openrgb fwupd; do
        printf '%-24s %s\n' "$s" "$(systemctl is-active "$s" 2>/dev/null)" >> "$R"
    done
fi

section "Kernel messages"
if have dmesg && dmesg >/dev/null 2>&1; then
    dmesg 2>/dev/null | grep -iE 'lenovo|ideapad|legion|wmi|platform.profile|firmware.attributes|048d|nvidia|thermal|acpi.*(error|warn)' | tail -n 200 >> "$R"
else
    echo "(not readable without sudo)" >> "$R"
fi

# ---------------- archive ----------------
archive="$USER_HOME/$(basename "$out").tar.gz"
if ! tar -czf "$archive" -C "$work" "$(basename "$out")" 2>/dev/null; then
    archive="$work/$(basename "$out").tar.gz"
    tar -czf "$archive" -C "$work" "$(basename "$out")"
fi
[ "$(id -u)" -eq 0 ] && [ "$USER_NAME" != root ] && chown "$USER_NAME": "$archive" 2>/dev/null
[ "$archive" = "$work/$(basename "$out").tar.gz" ] || rm -rf "$work"

kbd=$(grep -lis '048D' /sys/class/hidraw/hidraw*/device/uevent 2>/dev/null | wc -l)
drivers=$(have lsmod && lsmod | awk 'tolower($1) ~ /lenovo|ideapad|legion/ {print $1}' | tr '\n' ' ')
echo
echo "Model:           $(cat /sys/class/dmi/id/product_version 2>/dev/null) ($(cat /sys/class/dmi/id/product_name 2>/dev/null))"
echo "Kernel:          $(uname -r)"
echo "Profiles:        $(cat /sys/firmware/acpi/platform_profile_choices 2>/dev/null || echo none)"
echo "Lenovo drivers:  ${drivers:-none loaded}"
echo "ITE keyboard:    $kbd hidraw interface(s)"
echo
echo "Saved: $archive"
echo "A live session forgets files on shutdown: copy it to a USB stick before rebooting."
