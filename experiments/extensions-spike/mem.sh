#!/bin/zsh
# usage: mem.sh NAME [VAR=value...]  -> sums RSS (MB) of wisp + WebKit processes that appear during the run
SP=${0:A:h}; OUT=${SPIKE_OUT:-/tmp/wisp-spike}; mkdir -p $OUT/runs $OUT/homes; NAME=$1; shift
H=$OUT/homes/mem-$NAME; rm -rf $H; mkdir -p "$H/Library/Application Support/Wisp"
printf '{"workspace":0,"tabs":[{"url":"http://127.0.0.1:18931/login.html","title":"","workspace":0,"pinned":false,"active":true}]}' > "$H/Library/Application Support/Wisp/session.json"
cp $SP/config.json "$H/Library/Application Support/Wisp/config.json"
before=$(ps -axo pid=,comm= | grep -E "com.apple.WebKit" | awk '{print $1}' | sort)
cd ${0:A:h:h:h}
env HOME=$H WISP_SHOT=$OUT/runs/mem-$NAME.png WISP_SHOT_AT=14 "$@" ./target/debug/wisp > $OUT/runs/mem-$NAME.log 2>&1 &
W=$!; sleep 11
after=$(ps -axo pid=,comm= | grep -E "com.apple.WebKit" | awk '{print $1}' | sort)
new=$(comm -13 <(echo $before) <(echo $after) | tr '\n' ',' | sed 's/,$//')
ps -o pid=,rss=,comm= -p $W,$new | awk -v n=$NAME '{split($3,a,"/"); printf "  %-8s %6.0f MB %s\n", $1, $2/1024, a[length(a)]; t+=$2} END {printf "%s TOTAL %.0f MB in %d processes\n", n, t/1024, NR}'
wait $W
