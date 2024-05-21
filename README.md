# README
This project is thought as a simple example to show how to work with
network namespaces using the [rtnetlink](https://github.com/rust-netlink/rtnetlink)
Rust crate.

It will simply try to create a new network namespace and turn up the lo
interface.

## Usage
1) Build with cargo: `cargo build`
2) Run with sudo: `sudo ./target/debug/run_in_ns test`

If you do not want to run it as super user on your main system please
consider creating an unprivileged user namespace, like for example:

```bash
unshare -f --user --map-root-user --net  --mount /bin/bash
mkdir /tmp/netns
mount --bind /tmp/netns /var/run/netns
./target/debug/run_in_ns test
```

## Issue
Currently the program seems to deadlock during th retrieval of the lo
interface index:

```
./target/debug/run_in_ns test
[2024-05-21T13:54:16Z DEBUG run_in_ns] Net configuration PID: 321881
[2024-05-21T13:54:16Z DEBUG run_in_ns] ARE WE STOPPING YET???
[2024-05-21T13:54:16Z DEBUG netlink_proto::handle] handle: forwarding new request to connection
```
