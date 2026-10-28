use crate::{
    analytics,
    db::{Database, Error},
};
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub domain: String,
    pub id: String,
    pub time_ms: i64,
    pub scroll_depth: i64,
}

impl Event {
    pub fn validate(&mut self) -> bool {
        let Some(domain) = analytics::normalize_domain(&self.domain) else {
            return false;
        };
        self.domain = domain;
        self.id.len() == 32
            && hex::decode(&self.id).is_ok()
            && (0..=86_400_000).contains(&self.time_ms)
            && (0..=100).contains(&self.scroll_depth)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Collected {
    Stored,
    Duplicate,
    PendingPageview,
    Invalid,
}

pub async fn collect(
    db: &Database,
    mut event: Event,
    ua: String,
    clock: impl FnOnce() -> i64 + Send + 'static,
) -> Result<Collected, Error> {
    if !event.validate() {
        return Ok(Collected::Invalid);
    }
    if analytics::user_agent(&ua).is_none() {
        return Ok(Collected::Duplicate);
    }
    db.call(move |c| {
        let now=clock();
        let tx=c.transaction()?;
        let parent=tx.query_row("SELECT e.id,e.site_id,e.day,e.path,e.session_id,e.received_at FROM events e JOIN sites s ON s.id=e.site_id WHERE s.domain=?1 AND e.request_id=?2",params![event.domain,hex::decode(event.id).expect("validated ID")],|r| Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,i64>(4)?,r.get::<_,i64>(5)?))).optional()?;
        let Some((id,site,day,path,session,received))=parent else { return Ok(Collected::PendingPageview); };
        // A pageview's random ID acts only as a short-lived update capability.
        // Bound delayed/untrusted measurements by elapsed wall time and one day.
        if now < received || now-received > 86_400 { return Ok(Collected::Invalid); }
        let time=event.time_ms.min((now-received).saturating_add(1)*1000);
        let old=tx.query_row("SELECT time_ms,scroll_depth FROM page_engagement WHERE event_id=?1",[id],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?))).optional()?;
        if old.is_some_and(|(t,s)|time<=t && event.scroll_depth<=s) {return Ok(Collected::Duplicate);}
        let (old_time,old_scroll)=old.unwrap_or((0,0));
        let time=time.max(old_time); let scroll=event.scroll_depth.max(old_scroll);
        let (visits,previous_scroll)=tx.query_row("SELECT COUNT(*),COALESCE(MAX(g.scroll_depth),0) FROM events e JOIN page_engagement g ON g.event_id=e.id WHERE e.session_id=?1 AND e.path=?2",params![session,path],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?)))?;
        tx.execute("INSERT INTO page_engagement VALUES(?1,?2,?3) ON CONFLICT(event_id) DO UPDATE SET time_ms=excluded.time_ms,scroll_depth=excluded.scroll_depth",params![id,time,scroll])?;
        // Repeat views add active time but count the visit to this page once.
        // Scroll depth is the furthest point across those views, not their sum.
        tx.execute("INSERT INTO daily_engagement VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(site_id,day,path) DO UPDATE SET time_ms=time_ms+excluded.time_ms,visits=visits+excluded.visits,scroll_depth=scroll_depth+excluded.scroll_depth",params![site,day,path,time-old_time,i64::from(visits==0),scroll.max(previous_scroll)-previous_scroll])?;
        // Fresh engagement keeps an existing visit eligible for a subsequent
        // pageview. It does not change the last pageview, bounce or duration.
        // Retries return above; delayed measurements cannot revive old visits.
        tx.execute("UPDATE sessions SET active_at=?2 WHERE id=?1 AND active_at>?2-1860 AND active_at<=?2",params![session,now])?;
        tx.commit()?;
        Ok(Collected::Stored)
    }).await
}
