#!/bin/bash

for i in Yerus-cn Resonance-cn Catfish-cn Zinfandel-cn Mio-cn kp920-cn PowerNV-cn GreenGoo-cn Stomatopoda-cn PorterAlePro-cn; do 
    echo "$i" && ssh root@$i "cd /buildroots/buildit/buildit && git pull && systemctl restart buildit-worker.service";
done

ssh root@Yerus-cn "cd /buildroots/buildit-mips64r6el/buildit && git pull && systemctl restart buildit-worker.service"
