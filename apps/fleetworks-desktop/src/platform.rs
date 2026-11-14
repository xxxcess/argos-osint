//! Bounded macOS integration; no process runner, external IP service or deletion.

use std::net::IpAddr;
use std::net::Ipv4Addr;
use std::net::Ipv6Addr;
use std::net::ToSocketAddrs;
use std::net::UdpSocket;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use objc2_app_kit::NSWorkspace;
use objc2_foundation::NSArray;
use objc2_foundation::NSString;
use objc2_foundation::NSURL;
use url::Url;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeInterface {
    pub name: String,
    pub ip: IpAddr,
}

#[derive(Clone, Debug, Default)]
pub struct InterfaceSnapshot {
    pub interfaces: Vec<NativeInterface>,
    pub route_suggestion: Option<IpAddr>,
}

/// Called on a blocking worker, independently of the shell's working directory.
pub(crate) fn home() -> Result<PathBuf> {
    let path = PathBuf::from(objc2_foundation::NSHomeDirectory().to_string());
    anyhow::ensure!(
        path.is_absolute(),
        "macOS did not provide an absolute home directory"
    );
    Ok(path)
}

/// Called after the protected manager revalidates this exact owned child.
pub(crate) fn reveal(path: &Path) -> Result<()> {
    let path = path.to_str().context("workspace path is not valid UTF-8")?;
    let url = NSURL::fileURLWithPath_isDirectory(&NSString::from_str(path), true);
    NSWorkspace::sharedWorkspace()
        .activateFileViewerSelectingURLs(&NSArray::from_slice(&[url.as_ref()]));
    Ok(())
}

/// Generate opaque idempotency identities without persisting a new secret store.
pub(crate) fn request_id() -> Result<String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| anyhow::anyhow!("OS random source unavailable"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Enumerate current LAN/VPN addresses, with a suggestion from the gateway route.
/// UDP connect selects the local route without transmitting an application packet.
pub(crate) fn interfaces(origin: &str) -> Result<InterfaceSnapshot> {
    let mut head = std::ptr::null_mut();
    // SAFETY: getifaddrs initializes this pointer on success. A single guard frees
    // the linked allocation after all borrowed fields have been copied.
    anyhow::ensure!(
        unsafe { libc::getifaddrs(&mut head) } == 0,
        "cannot list local interfaces"
    );
    struct Addresses(*mut libc::ifaddrs);
    impl Drop for Addresses {
        fn drop(&mut self) {
            // SAFETY: this is the allocation returned by successful getifaddrs.
            unsafe { libc::freeifaddrs(self.0) };
        }
    }
    let _addresses = Addresses(head);
    let mut current = head;
    let mut rows = Vec::new();
    for _ in 0..4096 {
        if current.is_null() {
            break;
        }
        // SAFETY: each node and its address/name are borrowed only while the
        // getifaddrs allocation is alive; family selects the matching sockaddr.
        let entry = unsafe { &*current };
        current = entry.ifa_next;
        if entry.ifa_addr.is_null()
            || entry.ifa_name.is_null()
            || entry.ifa_flags & libc::IFF_UP as u32 == 0
            || entry.ifa_flags & libc::IFF_LOOPBACK as u32 != 0
        {
            continue;
        }
        let family = unsafe { (*entry.ifa_addr).sa_family } as i32;
        let ip = match family {
            libc::AF_INET => {
                let address = unsafe { &*entry.ifa_addr.cast::<libc::sockaddr_in>() };
                IpAddr::V4(Ipv4Addr::from(address.sin_addr.s_addr.to_ne_bytes()))
            }
            libc::AF_INET6 => {
                let address = unsafe { &*entry.ifa_addr.cast::<libc::sockaddr_in6>() };
                let ip = Ipv6Addr::from(address.sin6_addr.s6_addr);
                if ip.is_unicast_link_local() {
                    continue;
                }
                IpAddr::V6(ip)
            }
            _ => continue,
        };
        if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
            continue;
        }
        let name = unsafe { std::ffi::CStr::from_ptr(entry.ifa_name) }
            .to_string_lossy()
            .into_owned();
        let row = NativeInterface { name, ip };
        if !rows.contains(&row) && rows.len() < 256 {
            rows.push(row);
        }
    }
    rows.sort_by(|left, right| left.name.cmp(&right.name).then(left.ip.cmp(&right.ip)));
    let suggestion = route_address(origin).filter(|ip| rows.iter().any(|row| row.ip == *ip));
    Ok(InterfaceSnapshot {
        interfaces: rows,
        route_suggestion: suggestion,
    })
}

fn route_address(origin: &str) -> Option<IpAddr> {
    let origin = Url::parse(origin).ok()?;
    let host = origin.host_str()?;
    if host == "localhost" || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback()) {
        return None;
    }
    let port = origin.port_or_known_default()?;
    for peer in (host, port).to_socket_addrs().ok()?.take(8) {
        if peer.ip().is_loopback() {
            continue;
        }
        let bind = if peer.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        };
        let socket = UdpSocket::bind(bind).ok()?;
        if socket.connect(peer).is_ok() {
            return socket.local_addr().ok().map(|address| address.ip());
        }
    }
    None
}
