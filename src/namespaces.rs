use crate::errors::Errcode;
use crate::mountpoint::{bind_mount_namespace, create_directory};
// use crate::net::set_veth_up;

use nix::fcntl::{open, OFlag};
use nix::mount::{mount, MsFlags};
use nix::sched::{CloneFlags, unshare, setns};
use nix::unistd::{fork, ForkResult, Pid};
use nix::sys::wait::{waitpid, WaitStatus};
use nix::sys::stat::Mode;
use rtnetlink::{new_connection, NetworkNamespace};
use futures::TryStreamExt;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::exit;
use std::io::Write;
use std::os::unix::io::RawFd;

// This function will be called by the child during its configuration
// to create its namespace.
const UID_COUNT: u64 = 1;
const GID_COUNT: u64 = 1;
static NETNS: &str = "/run/netns/";

pub fn userns(real_uid: u32, real_gid: u32, target_uid: u32) -> Result<(), Errcode> {
    log::debug!("Switching to uid {} / gid {}...", target_uid, target_uid);

    if let Ok(mut uid_map) = File::create("/proc/self/uid_map") {
        if let Err(e) = uid_map.write_all(format!("{} {} {}", target_uid, real_uid, UID_COUNT).as_bytes()) {
            log::error!("Unable to open UID map: {:?}", e);
            return Err(Errcode::NamespacesError(format!("Unable to open UID Map: {}", e)));
        }
    } else {
        log::error!("Unable to create the UID MAP");
        return Err(Errcode::NamespacesError("Unable to create UID Map".to_string()));
    }

    if let Ok(mut setgroups) = OpenOptions::new().write(true).open("/proc/self/setgroups") {
        if let Err(e) = setgroups.write_all("deny".as_bytes()) {
            log::error!("Unable to write to setgroups: {:?}", e);
            return Err(Errcode::NamespacesError(format!("Unable to block setgroups: {}",e ))); }
    }

    if let Ok(mut gid_map) = File::create("/proc/self/gid_map") {
        if let Err(e) = gid_map.write_all(format!("{} {} {}", target_uid, real_gid, GID_COUNT).as_bytes()) {
            log::error!("Unable to open GID map: {:?}", e);
            return Err(Errcode::NamespacesError(format!("Unable to open GID Map: {}", e)));
        }
    } else {
        log::error!("Unable to create the GID MAP");
        return Err(Errcode::NamespacesError("Unable to create GID map".to_string()));
    }

    Ok(())
}

pub async fn open_namespace(ns_name: &String) -> Result<RawFd, Errcode> {

    let ns_path = PathBuf::from(format!("{}{}", NETNS, ns_name));

    // Use rnetlink to create namespace
    NetworkNamespace::add(ns_name.to_string()).await.map_err(|e| {
        Errcode::NamespacesError(format!{"Can not create network namespace {}: {}", ns_name, e})
    })?;


    match open(&ns_path, OFlag::empty(), Mode::empty()) {
        Ok(fd) => return Ok(fd),
        Err(e) => {
            log::error!("Can not create network namespace {}: {}", ns_name, e);
            return Err(Errcode::NamespacesError(format!("Can not create network namespace {}: {}", ns_name, e)));
        }
    }
}

pub fn mount_netns(hostname: &String) -> Result<(), Errcode> {
    let netns_mount = PathBuf::from(format!("/tmp/{}", hostname));
    create_directory(&netns_mount)?;
    let netns_dir = PathBuf::from(NETNS);
    // It's not mount(2) that I need to use
    if let Err(e) = bind_mount_namespace(&netns_mount, &netns_dir) {
        log::error!("Can not remount network namespace inside the container: {:?}", e);
        return Err(Errcode::NamespacesError(format!("Can not remount network namespace inside the container: {:?}", e)));
    }

    Ok(())

}

pub async fn run_in_namespace(ns_name: &String, veth_ip: &str, veth_2_ip: &str) -> Result<(), Errcode> {
    prep_for_fork()?;
    // Configure networking in the child namespace:
    // Fork a process that is set to the newly created namespace
    // Here set the veth ip addr, routing tables etc.
    // Unfortunately the NetworkNamespace interface of rtnetlink does
    // not offer these functionalities
    match unsafe { fork() } {
        Ok(ForkResult::Parent { child, .. }) => {
            // Parent process
            log::debug!("Net configuration PID: {}", child.as_raw());
            run_parent(child)
        }
        Ok(ForkResult::Child) => {
            // Child process
            // Move the child to the target namespace
            run_child(ns_name, veth_ip, veth_2_ip).await
            // NetworkNamespace::unshare_processing(format!("/run/netns/{}", ns_name))?;
            // set_lo_up().await;
            // std::process::exit(0);
        }
        Err(e) => {
            log::error!("Can not fork() for ns creation: {}", e);
            return Err(Errcode::NamespacesError(format!("Error fork(): {}",e)));
        }
    }

}

fn run_parent(child: Pid) -> Result<(), Errcode> {
    log::trace!("[Parent] Child PID: {}", child);
    match waitpid(child, None) {
        Ok(wait_status) => match wait_status {
            WaitStatus::Exited(_, res) => {
                log::trace!("Child exited with: {}", res);
                if res == 0 {
                    return Ok(());
                } else {
                    log::error!("Child exited with status {}", res);
                    return Err(Errcode::NamespacesError(format!("Namespace conf error: child exited with {}", res)));
                }
            }
            WaitStatus::Signaled(_, signal, coredump) => {
                log::error!("Child process killed by signal {signal} with core dump {coredump}");
                return Err(Errcode::NamespacesError(format!("Child process killed by signal {:?}", signal)));
            }
            _ => {
                log::error!("Unknown child process status: {:?}", wait_status);
                return Err(Errcode::NamespacesError(format!("Unknown child process status {:?}", wait_status)));
            }
        }
        Err(e) => {
            log::error!("wait error : {}", e);
            return Err(Errcode::NamespacesError(format!("Error during wait: {}", e)));
        }
    }

}

async fn run_child(ns_name: &String, veth_ip: &str, veth_2_ip: &str) -> Result<(), Errcode> {
    let res = split_namespace(ns_name, veth_ip, veth_2_ip).await;

    match res {
        Err(_) => {
            log::error!("Child process crashed");
            std::process::abort()
        }
        // Ok(Err(err)) => {
        //     log::error!("Child process failed");
        //     exit(1);
        // }
        Ok(()) => {
            log::debug!("Child exited normally");
            exit(0)
        }
    }
}

async fn split_namespace(ns_name: &String, veth_ip: &str, veth_2_ip: &str) -> Result<(), Errcode> {
    // Open NS path
    let ns_path = format!("{}{}", NETNS, ns_name);

    let mut open_flags = OFlag::empty();
    open_flags.insert(OFlag::O_RDONLY);
    open_flags.insert(OFlag::O_CLOEXEC);

    let fd = match open(Path::new(&ns_path), open_flags, Mode::empty()) {
        Ok(raw_fd) => raw_fd,
        Err(e) => {
            log::error!("Can not open network namespace: {}", e);
            return Err(Errcode::NamespacesError(format!("Can not open network namespace: {}", e)));
        }
    };
    // Switch to network namespace with CLONE_NEWNET
    if let Err(e) = setns(fd, CloneFlags::CLONE_NEWNET) {
        log::error!("Can not set namespace to target {}: {}", ns_name, e);
        return Err(Errcode::NamespacesError(format!("Unable to set target namespace: {}", e)));
    }
    // unshare with CLONE_NEWNS
    if let Err(e) = unshare(CloneFlags::CLONE_NEWNS) {
        log::error!("Can not unshare: {}", e);
        return Err(Errcode::NamespacesError(format!("Can not unshare: {}", e)));
    }
    // mount blind the fs
    let mut mount_flags = MsFlags::empty();
    mount_flags.insert(MsFlags::MS_REC);
    mount_flags.insert(MsFlags::MS_SLAVE);
    // let's
    //

    // call net_conf
    net_conf(ns_name, veth_ip, veth_2_ip).await?;

    Ok(())
}

// TODO need to open an issue to rtnetlink to find the proper way to configure an interface inside
// the created network namespace
async fn net_conf(ns_name: &String, veth_ip: &str, veth_2_ip: &str) -> Result<(), Errcode> {
    let mut lo_process = std::process::Command::new("ip")
        .args(["link", "set", "lo", "up"])
        .stdout(std::process::Stdio::null())
        .spawn()?;
    let veth_2 = format!("{}_peer", ns_name);
    let mut up_process = std::process::Command::new("ip")
        .args(["link", "set", veth_2.as_str(), "up"])
        .stdout(std::process::Stdio::null())
        .spawn()?;
    // set_veth_up().await?;
    let addr_subnet = format!("{}/24", veth_2_ip);
    let mut addr_process = std::process::Command::new("ip")
        .args(["addr", "add", addr_subnet.as_str(), "dev", veth_2.as_str()])
        .stdout(std::process::Stdio::null())
        .spawn()?;
    //
    let mut route_process = std::process::Command::new("ip")
        .args(["route", "add", "default", "dev", veth_2.as_str(), "via", veth_ip])
        .stdout(std::process::Stdio::null())
        .spawn()?;


    Ok(())
}

// TODO Unfortunately it seems that using rtnetlink inside the forked process that has been moved
// to the target network namespace hangs undefinitively.
async fn set_veth_up() -> Result<(), Errcode> {
    let (connection, handle, _) = new_connection()?;
    let mut links = handle.link().get().execute();
    'outer: while let Some(msg) = links.try_next().await? {
        for nla in msg.attributes.into_iter() {
            log::debug!("found link {}", msg.header.index);
            continue 'outer;
        }
    }
    let veth_idx = handle.link().get().match_name("test_veth".to_string()).execute().try_next().await?
                .ok_or_else(|| Errcode::NetworkError(format!("Can not find lo interface ")))?
                .header.index;
    log::debug!("LO INTERFACE INDEX: {}", veth_idx);
    handle.link().set(veth_idx).up().execute().await
         .map_err(|e| {Errcode::NetworkError(format!("Can not set lo interface up: {}", e))
     })?;
     Ok(())
}


// Cargo cult from the definition in rtnetlink
fn prep_for_fork() -> Result<(), Errcode> {
    Ok(())
}
