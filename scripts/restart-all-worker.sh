#!/bin/bash

for i in `cat servers.list`; do
    unset server_name server_port
    server_name=`awk '{ print $1 }' servers.list`
    server_port=`awk '{ print $2 }' servers.list`
    printf "Restarting BuildIt worker on $server_name ...    "
    ssh root@relay.aosc.io -p $i \
        "cd /buildroots/buildit/buildit && git pull && systemctl restart buildit-worker.service" \
            && printf "OK!\n" \
            || printf "Failed!\n"
    unset server_name server_port
done
