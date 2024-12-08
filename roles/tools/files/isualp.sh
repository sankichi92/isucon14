#!/bin/bash

set -eux

DISCORD_WEBHOOK="https://discord.com/api/webhooks/1314864227587981312/qv5PjNAh5xUiqvGIlUChpyBHK_8toi3tfNdm36L4clmd5Ss6ywywVMjWIFY0KoaML2L3"

sudo cat /var/log/nginx/access.log | alp ltsv -m \
"\
/assets/.+\$\
,/images/.+\$\
,/api/chair/rides/[^/]+/status\$\
,/api/app/rides/[^/]+/evaluation\$\
" \
--sort sum > /tmp/alp.txt
cat /tmp/alp.txt

curl -X POST -F username="alp[$(hostname)]" -F file=@/tmp/alp.txt "$DISCORD_WEBHOOK" &> /dev/null
