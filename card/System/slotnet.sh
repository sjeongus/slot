#!/bin/sh
# slot's home Wi-Fi: wpa_supplicant and udhcpc on wlan0.
#
# slot writes the supplicant config itself and drives this script from its network thread.
# Link play does not use this script; slot stops everything here before a link starts and
# brings it back after, so the two never share the radio.
#
#   slotnet.sh wifi start CONF   start, or reload CONF if already running
#   slotnet.sh wifi stop
#   slotnet.sh wifi alive        exit 0 if the supplicant is up
#   slotnet.sh wifi cli ARGS...  wpa_cli against slot's supplicant
#   slotnet.sh dhcp renew
set -u

NET="${AGS_NET_SYS:-/sys/class/net}"
RUN="${SLOTNET_RUN:-${SLOT_RUN:-/run}/slotnet}"
IFACE="${SLOTNET_IFACE:-wlan0}"
WPA_SUPPLICANT="${AGS_WPA_SUPPLICANT:-wpa_supplicant}"
WPA_CLI="${AGS_WPA_CLI:-wpa_cli}"
IP="${AGS_IP:-ip}"
UDHCPC="${AGS_UDHCPC:-udhcpc}"
DHCP_SCRIPT="${AGS_DHCP_SCRIPT:-/usr/share/udhcpc/default.script}"
# BaseOS loads the radio in the background and, on units that need a long power-off,
# only creates wlan0 several seconds into boot. NextUI waits 25 s; so does slot.
WAIT_IFACE="${SLOTNET_WAIT_IFACE:-100}"

nap() {
	sleep 0.25 2>/dev/null || sleep 1
}

pid_alive() {
	[ -f "$1" ] || return 1
	p="$(cat "$1" 2>/dev/null)"
	[ -n "$p" ] && [ -d "/proc/$p" ]
}

cli() {
	$WPA_CLI -p "$RUN" -i "$IFACE" "$@"
}

alive() {
	[ -e "$RUN/$IFACE" ] && cli ping 2>/dev/null | grep -q PONG
}

dhcp_start() {
	pid_alive "$RUN/udhcpc.pid" && return 0
	host="$(cat /proc/sys/kernel/hostname 2>/dev/null)"
	$UDHCPC -i "$IFACE" -b -S -R -p "$RUN/udhcpc.pid" -s "$DHCP_SCRIPT" \
		${host:+-x "hostname:$host"} >/dev/null 2>&1
}

dhcp_stop() {
	if pid_alive "$RUN/udhcpc.pid"; then
		kill "$(cat "$RUN/udhcpc.pid")" 2>/dev/null
	fi
	/bin/rm -f "$RUN/udhcpc.pid"
}

wifi_start() {
	conf="$1"
	[ -f "$conf" ] || return 1
	mkdir -p "$RUN"
	if alive; then
		cli reconfigure >/dev/null 2>&1 || return 1
		dhcp_start
		return 0
	fi
	/bin/rm -f "$RUN/$IFACE"
	w=0
	while [ ! -d "$NET/$IFACE" ] && [ "$w" -lt "$WAIT_IFACE" ]; do
		nap
		w=$((w + 1))
	done
	[ -d "$NET/$IFACE" ] || return 1
	rfkill unblock wifi 2>/dev/null || true
	$IP link set "$IFACE" up 2>/dev/null || true
	$WPA_SUPPLICANT -B -i "$IFACE" -c "$conf" -Dnl80211 -P "$RUN/wpa.pid" >/dev/null 2>&1 || return 1
	s=0
	while [ ! -e "$RUN/$IFACE" ] && [ "$s" -lt 20 ]; do
		nap
		s=$((s + 1))
	done
	[ -e "$RUN/$IFACE" ] || return 1
	dhcp_start
	return 0
}

wifi_stop() {
	dhcp_stop
	if [ -e "$RUN/$IFACE" ]; then
		cli terminate >/dev/null 2>&1
		s=0
		while [ -e "$RUN/$IFACE" ] && [ "$s" -lt 8 ]; do
			nap
			s=$((s + 1))
		done
	fi
	if pid_alive "$RUN/wpa.pid"; then
		kill "$(cat "$RUN/wpa.pid")" 2>/dev/null
	fi
	/bin/rm -f "$RUN/$IFACE" "$RUN/wpa.pid"
	$IP addr flush dev "$IFACE" 2>/dev/null || true
	return 0
}

dhcp_renew() {
	if pid_alive "$RUN/udhcpc.pid"; then
		kill -USR1 "$(cat "$RUN/udhcpc.pid")" 2>/dev/null
	else
		dhcp_start
	fi
}

case "${1:-} ${2:-}" in
"wifi start") wifi_start "${3:-}" ;;
"wifi stop") wifi_stop ;;
"wifi alive") alive ;;
"wifi cli")
	shift 2
	cli "$@"
	;;
"dhcp renew") dhcp_renew ;;
*)
	echo "usage: slotnet.sh wifi [start CONF|stop|alive|cli ARGS...] | dhcp renew" >&2
	exit 2
	;;
esac
