#!/bin/bash

set -eux

DISCORD_WEBHOOK="https://discord.com/api/webhooks/1314864227587981312/qv5PjNAh5xUiqvGIlUChpyBHK_8toi3tfNdm36L4clmd5Ss6ywywVMjWIFY0KoaML2L3"

sudo mysqldumpslow /var/log/mysql/slow.log -s t > /tmp/mysqldumpslow.txt
cat /tmp/mysqldumpslow.txt

curl -X POST -F username="mysqldumpslow[$(hostname)]" -F file=@/tmp/mysqldumpslow.txt "$DISCORD_WEBHOOK" &> /dev/null
