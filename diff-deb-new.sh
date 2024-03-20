#!/bin/sh
# Usage: diff-deb-new.sh right.deb
right=$(mktemp /tmp/diff-deb.XXXXXX)
dpkg --contents $1 | awk '!($2=$3=$4=$5="")' > $right
touch -m -d "1980-01-01" $right

diff --label a --label b -u /dev/null $right
