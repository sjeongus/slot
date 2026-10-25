#!/bin/sh
set -eu

HERE="$(cd "$(dirname "$0")" && pwd)"
SCRIPT="$HERE/System/slotnet.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT HUP INT TERM
FAILS=0

fail() {
	echo "FAIL: $*"
	FAILS=$((FAILS + 1))
}

make_fake() {
	name="$1"
	out="$2"
	shift 2
	{
		echo '#!/bin/sh'
		echo "printf '%s\\n' \"\$*\" >> \"$out\""
		for line in "$@"; do echo "$line"; done
		echo 'exit 0'
	} > "$TMP/bin/$name"
	chmod 755 "$TMP/bin/$name"
}

reset() {
	rm -rf "$TMP/net" "$TMP/run" "$TMP"/*.log "$TMP/bin"
	mkdir -p "$TMP/net/wlan0" "$TMP/run" "$TMP/bin"
	echo 'ctrl_interface=x' > "$TMP/wpa.conf"
	# A supplicant that "starts" by creating its control socket and pid file.
	make_fake wpa_supplicant "$TMP/sup.log" \
		': > "$SLOTNET_RUN/wlan0"' \
		'sleep 30 & echo $! > "$SLOTNET_RUN/wpa.pid"'
	make_fake wpa_cli "$TMP/cli.log" \
		'case "$*" in *" ping") [ -e "$SLOTNET_RUN/wlan0" ] && echo PONG ;; *" terminate") rm -f "$SLOTNET_RUN/wlan0"; echo OK ;; *) echo OK ;; esac'
	make_fake udhcpc "$TMP/dhcp.log" \
		'sleep 30 & while [ $# -gt 0 ]; do [ "$1" = -p ] && echo $! > "$2"; shift; done'
	make_fake ip "$TMP/ip.log"
}

run() {
	PATH="$TMP/bin:$PATH" \
		AGS_NET_SYS="$TMP/net" SLOTNET_RUN="$TMP/run" SLOTNET_WAIT_IFACE=1 \
		AGS_WPA_SUPPLICANT="$TMP/bin/wpa_supplicant" AGS_WPA_CLI="$TMP/bin/wpa_cli" \
		AGS_IP="$TMP/bin/ip" AGS_UDHCPC="$TMP/bin/udhcpc" AGS_DHCP_SCRIPT=/dev/null \
		sh "$SCRIPT" "$@"
}

cleanup() {
	for p in "$TMP/run/wpa.pid" "$TMP/run/udhcpc.pid"; do
		[ -f "$p" ] && kill "$(cat "$p")" 2>/dev/null || true
	done
}

reset
run wifi alive && fail "alive before start"
run wifi start "$TMP/wpa.conf" || fail "start failed"
grep -q -- "-i wlan0 -c $TMP/wpa.conf" "$TMP/sup.log" || fail "supplicant not given the config"
grep -q "link set wlan0 up" "$TMP/ip.log" || fail "wlan0 not raised"
grep -q -- "-i wlan0 -b" "$TMP/dhcp.log" || fail "dhcp not started in the background"
run wifi alive || fail "not alive after start"

: > "$TMP/sup.log"
run wifi start "$TMP/wpa.conf" || fail "second start failed"
[ -s "$TMP/sup.log" ] && fail "second start launched another supplicant"
grep -q "reconfigure" "$TMP/cli.log" || fail "second start did not reload the config"
[ "$(wc -l < "$TMP/dhcp.log")" = 1 ] || fail "second start launched another dhcp client"

out="$(run wifi cli status)"
[ "$out" = OK ] || fail "cli output not passed through: $out"
grep -q -- "-p $TMP/run -i wlan0 status" "$TMP/cli.log" || fail "cli not pointed at slot's socket"

run dhcp renew || fail "renew failed"

run wifi stop
run wifi alive && fail "alive after stop"
grep -q "terminate" "$TMP/cli.log" || fail "supplicant not terminated"
[ -f "$TMP/run/udhcpc.pid" ] && fail "dhcp pid left behind"
grep -q "addr flush dev wlan0" "$TMP/ip.log" || fail "address not flushed"
cleanup

reset
rm -rf "$TMP/net/wlan0"
run wifi start "$TMP/wpa.conf" && fail "started without wlan0"
cleanup

reset
run wifi start "$TMP/missing.conf" && fail "started without a config"

run bogus 2>/dev/null && fail "unknown verb accepted"

if [ "$FAILS" -gt 0 ]; then
	echo "$FAILS slotnet test(s) failed"
	exit 1
fi
echo "slotnet: ok"
