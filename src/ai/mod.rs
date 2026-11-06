//! AI judgment via Jev. Rules should prefer these helpers over hand-rolled heuristics:
//! Jev is cheap and fast enough to run on every message.

pub mod jev;

use anyhow::Result;
use async_trait::async_trait;
use poise::serenity_prelude::Message;
use serde_json::{Value, json};

use crate::app::Data;
use jev::{Answers, Image, Jev, Question, normalize_image, noul};

/// A yes/no answer (noul) at or above this counts as "yes". The one place it's decided.
pub const YES: f32 = 0.5;

/// Answers named questions about some `state` (plus images it references) in one request.
/// [`Jev`] in production; a fake in tests.
#[async_trait]
pub trait Judge: Send + Sync {
    async fn ask(
        &self,
        state: Value,
        questions: Vec<(&'static str, Question)>,
        images: Vec<Image>,
    ) -> Result<Answers>;
}

#[async_trait]
impl Judge for Jev {
    async fn ask(
        &self,
        state: Value,
        questions: Vec<(&'static str, Question)>,
        images: Vec<Image>,
    ) -> Result<Answers> {
        Jev::ask(self, state, questions, images).await
    }
}

/// Max images per message sent to Jev (keeps requests small and fast).
const MAX_IMAGES: usize = 4;
/// Skip anything bigger than Jev's per-image limit before even downloading.
const MAX_IMAGE_BYTES: u32 = 16 * 1024 * 1024;

/// Download a message's image attachments and embed previews (e.g. Tenor GIFs), normalized for Jev.
/// Failures are skipped: a broken image just means Jev judges the text alone.
pub async fn message_images(msg: &Message) -> Vec<Image> {
    let mut urls: Vec<String> = msg
        .attachments
        .iter()
        .filter(|a| {
            a.content_type
                .as_deref()
                .is_some_and(|t| t.starts_with("image/"))
        })
        .filter(|a| a.size <= MAX_IMAGE_BYTES)
        .map(|a| a.url.clone())
        .collect();
    for e in &msg.embeds {
        if let Some(url) = e
            .image
            .as_ref()
            .map(|i| i.url.clone())
            .or_else(|| e.thumbnail.as_ref().map(|t| t.url.clone()))
        {
            urls.push(url);
        }
    }
    urls.truncate(MAX_IMAGES);

    let mut out = Vec::new();
    for (i, url) in urls.into_iter().enumerate() {
        match fetch_image(&url).await {
            Ok(jpeg) => out.push(Image {
                placeholder: format!("image-{}-{}", msg.id, i + 1),
                jpeg,
            }),
            Err(e) => tracing::debug!("skipping image {url}: {e:#}"),
        }
    }
    out
}

async fn fetch_image(url: &str) -> anyhow::Result<Vec<u8>> {
    let resp = reqwest::get(url).await?.error_for_status()?;
    let bytes = resp.bytes().await?;
    anyhow::ensure!(bytes.len() <= MAX_IMAGE_BYTES as usize, "too large");
    tokio::task::spawn_blocking(move || normalize_image(&bytes)).await?
}

/// Jev `state` for a chat message: plain text, or text plus image placeholders.
pub fn message_state(text: &str, images: &[Image]) -> Value {
    if images.is_empty() {
        json!(text)
    } else {
        let refs: Vec<&str> = images.iter().map(|i| i.placeholder.as_str()).collect();
        json!({ "message_text": text, "attached_images": refs })
    }
}

/// Ask a single yes/no question. `None` if no judge is configured or the call failed (logged).
pub async fn ask_noul(
    data: &Data,
    state: Value,
    images: Vec<Image>,
    question: &str,
    criteria: Option<(&str, &str)>,
) -> Option<f32> {
    let judge = data.judge.as_deref()?;
    match judge
        .ask(state, vec![("q", noul(question, criteria))], images)
        .await
    {
        Ok(a) => a.noul("q").ok(),
        Err(e) => {
            tracing::warn!("jev call failed: {e:#}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_shapes() {
        assert_eq!(message_state("hi", &[]), json!("hi"));
        let img = Image {
            placeholder: "image-1-1".into(),
            jpeg: vec![],
        };
        assert_eq!(
            message_state("hi", &[img]),
            json!({ "message_text": "hi", "attached_images": ["image-1-1"] })
        );
    }
}
