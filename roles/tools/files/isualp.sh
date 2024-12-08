#!/bin/bash

set -eux

DISCORD_WEBHOOK="https://discord.com/api/webhooks/1314864227587981312/qv5PjNAh5xUiqvGIlUChpyBHK_8toi3tfNdm36L4clmd5Ss6ywywVMjWIFY0KoaML2L3"

cat /var/log/nginx/access.log | alp ltsv -m \
"\
/api/user/[^/]+/icon,\
/api/user/[^/]+/theme,\
/api/user/[^/]+/statistics,\
/api/livestream/[0-9]+\$,\
/api/livestream/[0-9]+/statistics,\
/api/livestream/[0-9]+/livecomment,\
/api/livestream/[0-9]+/reaction,\
/api/livestream/[0-9]+/report,\
/api/livestream/[0-9]+/ngwords,\
/api/livestream/[0-9]+/exit,\
/api/livestream/[0-9]+/moderate,\
/api/livestream/[0-9]+/enter\
" \
--sort sum > /tmp/alp.txt
cat /tmp/alp.txt

curl -X POST -F username="alp[$(hostname)]" -F file=@/tmp/alp.txt "$DISCORD_WEBHOOK" &> /dev/null
