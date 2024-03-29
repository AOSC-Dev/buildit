#!/bin/bash

while read i; do
    unset server_name server_port
    server_name=$(echo $i | awk '{ print $1 }')
    server_port=$(echo $i | awk '{ print $2 }')
    case $1 in
        restart)
            printf "Restarting BuildIt worker on $server_name ($server_port) ... "
            # ssh -n disables reading from stdin, which overrides the while read loop.
            ssh -n root@relay-cn.aosc.io -p $server_port \
                "cd /buildroots/buildit/buildit && git pull -q && systemctl restart buildit-worker.service" \
                    && printf "OK!\n" \
                    || printf "Failed to restart BuildIt worker on $server_name ($server_port)!\n"
            ;;
        stop)
            printf "Stopping BuildIt worker on $server_name ($server_port) ... "
            # ssh -n disables reading from stdin, which overrides the while read loop.
            ssh -n root@relay-cn.aosc.io -p $server_port \
                "systemctl stop buildit-worker.service" \
                    && printf "OK!\n" \
                    || printf "Failed to restart BuildIt worker on $server_name ($server_port)!\n"
            ;;
        *)
            echo "Invalid operation specified! (restart or stop?)"
            exit 1
            ;;
    esac
    unset server_name server_port
done < servers.list
