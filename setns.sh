#!/bin/bash

#NETNS=nft-$(cat /proc/sys/kernel/random/uuid)
NETNS=test
KEYWORD=$(echo provastart | tr a-z A-Z)
NOARGS=0

LINE_NUM=$(awk -v needle=KEYWORD '/needle/{print NR}' $0)
UNSHARE=/usr/bin/unshare
NS_DIR=/tmp/$NETNS

function clean {
    ip netns del $NETNS
    umount /run/netns
    umount /var/run
    rm -rf $NS_DIR
    echo "CLEAN"
}

function rootless_ns {
    mkdir $NS_DIR
    mount --bind $NS_DIR /var/run/
    ip netns add $NETNS
    trap clean EXIT
    ip link add test type veth peer test2
    ip link set test netns test
    bash
}

function unshare {
    $UNSHARE -f --user --map-root-user --net  --mount /bin/bash -c "$0 rootless_ns"
}

if [ $# -eq 0 ]; then
    echo $0 " + " $1
    unshare
else
    echo $0 " - " $1
fi

"$@"
