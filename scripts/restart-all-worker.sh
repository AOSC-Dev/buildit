#!/bin/bash

for i in Yerus-cn Resonance-cn Catfish-cn Mio-cn kp920-cn eleventh PowerNV-cn GreenGoo-cn Stomatopoda-cn PorterAlePro-cn Taple-cn; do 
    echo "$i" && ssh root@$i "cd /buildroots/buildit/buildit && git pull && systemctl restart buildit-worker.service";
done
