#!/usr/bin/env bash
# T24-29: disposable second ext4 device, confined to the isolated CI task root.
set -euo pipefail
umask 077
root="$(realpath -e -- "${1:?isolated foundation root}")"
test "${GITHUB_ACTIONS:-}" = true
test "${RUNNER_OS:-}" = Linux
test -d "$root/source"
test -d "$root/receipts"
test "$(stat -c %u "$root")" = "$(id -u)"
test "$(stat -c %a "$root")" = 700
fixture="$root/native-volume-fixture"
# Never reuse an image, mount directory or device from a previous invocation.
test ! -e "$fixture"
mkdir -m 700 -- "$fixture"
fixture="$(realpath -e -- "$fixture")"
test "$fixture" = "$root/native-volume-fixture"
image="$fixture/ext4.img"
mountpoint="$fixture/mounted"
mkdir -m 700 -- "$mountpoint" "$fixture/source"
mountpoint="$(realpath -e -- "$mountpoint")"
test "$mountpoint" = "$root/native-volume-fixture/mounted"
test ! -e "$image"
(set -o noclobber; : > "$image")
test -f "$image" && test ! -L "$image"
test "$(stat -c %u "$image")" = "$(id -u)"
test "$(stat -c %h "$image")" = 1
truncate -s 256M -- "$image"
loop=''
mounted=false
cleanup() {
  local status=$?
  trap - EXIT
  if $mounted; then sudo umount -- "$mountpoint" || status=1; fi
  # umount autoclears the loop device; detach only if it is still attached.
  if [[ -n "$loop" ]] && sudo losetup -- "$loop" >/dev/null 2>&1; then sudo losetup -d -- "$loop" || status=1; fi
  printf '%s\n' "$status" > "$root/receipts/native-fixtures.exit"
  exit "$status"
}
trap cleanup EXIT
# mkfs receives only the fresh regular file, never a computed live disk device.
mkfs.ext4 -q -F -- "$image"
loop="$(sudo losetup --find --show -- "$image")"
test "$(sudo losetup -n -O BACK-FILE -- "$loop")" = "$image"
sudo mount -t ext4 -o nodev,nosuid,noexec -- "$loop" "$mountpoint"
mounted=true
sudo chown "$(id -u):$(id -g)" -- "$mountpoint"
chmod 700 -- "$mountpoint"
test "$(findmnt -n -o FSTYPE -T "$mountpoint")" = ext4
test "$(stat -c %d "$fixture/source")" != "$(stat -c %d "$mountpoint")"
export FOUNDATION_CROSS_VOLUME_SOURCE="$fixture/source"
export FOUNDATION_CROSS_VOLUME_DEST="$mountpoint"
export HOME="$root/test-home" USERPROFILE="$root/test-home"
export XDG_DATA_HOME="$HOME/.local/share" XDG_CONFIG_HOME="$HOME/.config"
cd "$root/source"
printf 'scope=T24-29 native cross-volume fixture\ncandidate=%s\n' "$(cat "$root/receipts/candidate.commit")" > "$root/receipts/native-fixtures.log"
findmnt -T "$mountpoint" -o FSTYPE,OPTIONS >> "$root/receipts/native-fixtures.log"
cargo test -p dendro-core --locked --test native_foundation_acceptance cross_volume_closed_copy_then_local_pointer_publication -- --ignored --exact --nocapture 2>&1 | tee -a "$root/receipts/native-fixtures.log"
grep -Fq 'T24-29 actual distinct-device copy, private destination, unchanged closed source and verified local pointer: passed' "$root/receipts/native-fixtures.log"
grep -Fq '1 passed; 0 failed' "$root/receipts/native-fixtures.log"
