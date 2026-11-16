#!/bin/zsh
# usage: run.sh NAME URL [VAR=value ...]   (WISP_EXTENSIONS etc. passed as VAR=value)
SP=${0:A:h}; OUT=${SPIKE_OUT:-/tmp/wisp-spike}; mkdir -p $OUT/runs $OUT/homes; NAME=$1; URL=$2; shift 2
H=$OUT/homes/$NAME; rm -rf $H; mkdir -p "$H/Library/Application Support/Wisp"
printf '{"workspace":0,"tabs":[{"url":"%s","title":"","workspace":0,"pinned":false,"active":true}]}' "$URL" > "$H/Library/Application Support/Wisp/session.json"
cp ${WISP_CONFIG:-$SP/config.json} "$H/Library/Application Support/Wisp/config.json"
cd ${0:A:h:h:h}
env HOME=$H WISP_SHOT=$OUT/runs/$NAME.png "$@" ./target/debug/wisp > $OUT/runs/$NAME.log 2>&1
grep -E "wisp-ext|wisp-probe|screenshot|panic|error" $OUT/runs/$NAME.log | cut -c1-400
