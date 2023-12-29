#!/bin/bash

while read i; do
    unset server_name server_port
    server_name=$(echo $i | awk '{ print $1 }')
    server_port=$(echo $i | awk '{ print $2 }')
    printf "Restarting BuildIt worker on $server_name ($server_port) ... "
    ssh root@relay.aosc.io -p $server_port \
        "cd /buildroots/buildit/buildit && git pull && systemctl restart buildit-worker.service" \
            && printf "OK!\n" \
            || printf "Failed!\n"
    unset server_name server_port
done < servers.list
