//! KDE Connect device discovery and SFTP mount over D-Bus (zbus).

/// A connected phone: device id + SFTP mount point.
#[derive(Debug, Clone)]
pub struct Phone {
    pub device_id: String,
    pub mount_point: std::path::PathBuf,
}

/// Pick the first reachable AND paired device id from a listing.
pub fn pick_device(names: &[(String, bool, bool)]) -> Option<String> {
    names
        .iter()
        .find(|(_, reachable, paired)| *reachable && *paired)
        .map(|(id, _, _)| id.clone())
}

/// Music dir inside the phone for an explicit override value (pure, testable).
pub fn music_dir_in(phone: &Phone, phone_music_dir: &str) -> std::path::PathBuf {
    phone.mount_point.join(phone_music_dir)
}

/// Music dir using the FLACAST_PHONE_MUSIC_DIR env var (default like flacast).
pub fn music_dir(phone: &Phone) -> std::path::PathBuf {
    music_dir_in(
        phone,
        &std::env::var("FLACAST_PHONE_MUSIC_DIR")
            .unwrap_or_else(|_| "storage/emulated/0/Media/Music".into()),
    )
}

const DEST: &str = "org.kde.kdeconnect";
const SFTP_IFACE: &str = "org.kde.kdeconnect.device.sftp";

/// Find the first reachable paired device and mount its SFTP share.
pub async fn connect() -> anyhow::Result<Phone> {
    match connect_on_bus(None).await {
        Ok(p) => Ok(p),
        Err(_) => {
            // Fallback: sessions started via dbus-launch can't see the
            // systemd user bus where KDE Connect registers (flacast lesson).
            match systemd_bus_address() {
                Some(addr) => connect_on_bus(Some(&addr)).await,
                None => Err(anyhow::anyhow!(
                    "no reachable phone found and no systemd user bus available"
                )),
            }
        }
    }
}

/// Explicit systemd user bus address, if it exists as a socket.
fn systemd_bus_address() -> Option<String> {
    let runtime = std::env::var("XDG_RUNTIME_DIR").ok()?;
    let addr = format!("unix:path={runtime}/bus");
    std::path::Path::new(&format!("{runtime}/bus")).exists().then_some(addr)
}

async fn connect_on_bus(addr: Option<&str>) -> anyhow::Result<Phone> {
    let conn = match addr {
        Some(a) => {
            use std::str::FromStr;
            zbus::connection::Builder::address(zbus::address::Address::from_str(a)?)
                .build()
                .await?
        }
        None => zbus::Connection::session().await?,
    };
    // org.kde.kdeconnect.daemon on /modules/kdeconnect:
    // deviceNames(onlyReachable, onlyPaired) -> a{ss}
    let names: std::collections::HashMap<String, String> = conn
        .call_method(
            Some(DEST),
            "/modules/kdeconnect",
            Some("org.kde.kdeconnect.daemon"),
            "deviceNames",
            &(true, true),
        )
        .await?
        .body()
        .deserialize()?;

    let device_id = pick_device(
        &names
            .keys()
            .map(|k| (k.clone(), true, true))
            .collect::<Vec<_>>(),
    )
    .ok_or_else(|| anyhow::anyhow!(
        "no reachable phone found. open the KDE Connect app on your phone and check both devices share a network"
    ))?;

    let path = format!("/modules/kdeconnect/devices/{device_id}/sftp");
    let sftp = zbus::Proxy::new(&conn, DEST, path, SFTP_IFACE).await?;

    // NOTE: isMounted/mountPoint are METHODS on this interface (not properties),
    // and mountAndWait returns a bool — verified against the live daemon.
    let mounted: bool = sftp.call("isMounted", &()).await.unwrap_or(false);
    if !mounted {
        let _started: bool = sftp.call("mountAndWait", &()).await?;
        // the share can take a moment to become browsable (flacast lesson)
        for _ in 0..10 {
            if sftp.call::<_, _, bool>("isMounted", &()).await.unwrap_or(false) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }

    let mount_point: String = sftp.call("mountPoint", &()).await?;
    Ok(Phone {
        device_id,
        mount_point: mount_point.into(),
    })
}

/// Unmount the phone's SFTP share. Uses the same bus fallback as connect(),
/// because the farewell runs after the TUI in whatever bus env the user has.
pub async fn unmount(device_id: &str) -> anyhow::Result<()> {
    use std::str::FromStr;
    let conn = match zbus::Connection::session().await {
        Ok(c) => c,
        Err(_) => match systemd_bus_address() {
            Some(a) => {
                zbus::connection::Builder::address(zbus::address::Address::from_str(&a)?)
                    .build()
                    .await?
            }
            None => anyhow::bail!("no D-Bus session available"),
        },
    };
    let path = format!("/modules/kdeconnect/devices/{device_id}/sftp");
    let sftp = zbus::Proxy::new(&conn, DEST, path, SFTP_IFACE).await?;
    sftp.call::<_, _, ()>("unmount", &()).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_device_prefers_reachable_and_paired() {
        let list = vec![
            ("dead".to_string(), false, true),
            ("good".to_string(), true, true),
        ];
        assert_eq!(pick_device(&list), Some("good".to_string()));
    }

    #[test]
    fn pick_device_skips_unpaired() {
        let list = vec![("newphone".to_string(), true, false)];
        assert_eq!(pick_device(&list), None);
    }

    #[test]
    fn pick_device_empty_is_none() {
        assert_eq!(pick_device(&[]), None);
    }

    #[test]
    fn music_dir_default_and_override() {
        let phone = Phone {
            device_id: "abc".into(),
            mount_point: "/run/user/1000/dev".into(),
        };
        assert_eq!(
            music_dir_in(&phone, "storage/emulated/0/Media/Music"),
            std::path::PathBuf::from("/run/user/1000/dev/storage/emulated/0/Media/Music")
        );
        assert_eq!(
            music_dir_in(&phone, "Music"),
            std::path::PathBuf::from("/run/user/1000/dev/Music")
        );
    }
}
