//! Short-lived, site-scoped identities. Only an active visit bridges UTC rotation.
use hmac::{Hmac, Mac};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::Sha256;
use std::net::IpAddr;

fn hash(secret: &[u8], site: i64, ip: IpAddr, ua: &str) -> [u8; 16] {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC accepts any key size");
    mac.update(&site.to_be_bytes());
    mac.update(ip.to_string().as_bytes());
    mac.update(&[0]);
    mac.update(ua.as_bytes());
    mac.finalize().into_bytes()[..16].try_into().unwrap()
}

pub(crate) fn identify(
    c: &Connection,
    site: i64,
    now: i64,
    random: &[u8; 32],
    ip: IpAddr,
    ua: &str,
) -> rusqlite::Result<[u8; 16]> {
    let day = chrono::DateTime::from_timestamp(now, 0)
        .expect("current timestamp")
        .date_naive()
        .to_string();
    let expiry = now.div_euclid(86400) * 86400 + 172800;
    let stored = c.query_row(
        "SELECT day,secret,expires_at,previous_secret,previous_expires_at FROM site_salts WHERE site_id=?1",
        [site],
        |r| Ok((r.get::<_,String>(0)?,r.get::<_,Vec<u8>>(1)?,r.get::<_,i64>(2)?,r.get::<_,Option<Vec<u8>>>(3)?,r.get::<_,Option<i64>>(4)?)),
    ).optional()?;
    let (secret, previous) = if let Some((_, secret, _, previous, previous_expiry)) = stored
        .as_ref()
        .filter(|(stored_day, _, expires, _, _)| *stored_day == day && *expires > now)
    {
        (
            secret.clone(),
            previous
                .clone()
                .filter(|_| previous_expiry.is_some_and(|end| end > now)),
        )
    } else {
        let previous = stored.filter(|(_, _, expires, _, _)| *expires > now);
        let previous_secret = previous
            .as_ref()
            .map(|(_, secret, _, _, _)| secret.as_slice());
        let previous_expiry = previous.as_ref().map(|(_, _, expiry, _, _)| *expiry);
        c.execute(
            "INSERT INTO site_salts(site_id,day,secret,expires_at,previous_secret,previous_expires_at) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(site_id) DO UPDATE SET day=excluded.day,secret=excluded.secret,expires_at=excluded.expires_at,previous_secret=excluded.previous_secret,previous_expires_at=excluded.previous_expires_at",
            params![site, day, random.as_slice(), expiry, previous_secret, previous_expiry],
        )?;
        (random.to_vec(), previous.map(|(_, secret, _, _, _)| secret))
    };
    let current = hash(&secret, site, ip, ua);
    let Some(previous) = previous else {
        return Ok(current);
    };
    // Current-day visits take priority. An active prior-day visit keeps its
    // original identity and attribution; expired visits use today's identity.
    for candidate in [current, hash(&previous, site, ip, ua)] {
        let active: bool = c.query_row(
            "SELECT EXISTS(SELECT 1 FROM sessions INDEXED BY sessions_visitor_time WHERE site_id=?1 AND visitor=?2 AND active_at>?3-1860)",
            params![site, candidate.as_slice(), now], |r| r.get(0),
        )?;
        if active {
            return Ok(candidate);
        }
    }
    Ok(current)
}

pub(crate) fn prune(c: &Connection, now: i64) -> rusqlite::Result<()> {
    c.execute("UPDATE site_salts SET previous_secret=NULL,previous_expires_at=NULL WHERE previous_expires_at<=?1", [now])?;
    c.execute("DELETE FROM site_salts WHERE expires_at<=?1", [now])?;
    Ok(())
}
