#!/bin/sh
# Usage: diff-deb.sh left.deb right.deb
left=$(mktemp /tmp/diff-deb.XXXXXX)
dpkg --contents $1 | awk '!($2=$3=$4=$5="")' > $left
touch -m -d "1980-01-01" $left

right=$(mktemp /tmp/diff-deb.XXXXXX)
dpkg --contents $2 | awk '!($2=$3=$4=$5="")' > $right
touch -m -d "1980-01-01" $right

diff --label a --label b -u $left $right
