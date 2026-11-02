use crate::types::Message;
use std::collections::VecDeque;

#[derive(Debug, Clone)]
pub struct Session {
    pub key: String,
    pub count: usize,
    pub url: Option<String>,
    pub retryable: bool,
}

#[derive(Debug, Default)]
pub struct SessionStore {
    entries: VecDeque<Session>,
    max: usize,
}

impl SessionStore {
    pub fn new(max: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            max,
        }
    }

    pub fn key(system: &str, first_user: &str) -> String {
        format!("{}\n{}", norm(system), norm(first_user))
    }

    pub fn find(
        &self,
        key: &str,
        count: usize,
        messages: &[Message],
    ) -> Option<(usize, Option<String>, Vec<Message>)> {
        for (i, s) in self.entries.iter().enumerate().rev() {
            if s.key != key {
                continue;
            }
            if count > s.count {
                return Some((i, s.url.clone(), messages[s.count..].to_vec()));
            }
            if count + 1 == s.count && s.retryable {
                return Some((i, s.url.clone(), messages[messages.len() - 1..].to_vec()));
            }
            return None;
        }
        None
    }

    pub fn save(
        &mut self,
        index: Option<usize>,
        key: String,
        count: usize,
        url: Option<String>,
        retryable: bool,
    ) {
        if let Some(i) = index {
            self.entries.remove(i);
        }
        self.entries.push_back(Session {
            key,
            count,
            url,
            retryable,
        });
        while self.entries.len() > self.max {
            self.entries.pop_front();
        }
    }
}

fn norm(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Block;

    fn msg(t: &str) -> Message {
        Message {
            role: "user".into(),
            content: vec![Block::Text { text: t.into() }],
        }
    }

    #[test]
    fn continues_with_only_new_messages() {
        let mut store = SessionStore::new(20);
        let key = SessionStore::key("sys", "first");
        store.save(
            None,
            key.clone(),
            3,
            Some("https://chat.deepseek.com/a/chat/s/x".into()),
            true,
        );
        let msgs = vec![msg("first"), msg("a"), msg("b"), msg("c")];
        let (_, url, rest) = store.find(&key, 4, &msgs).unwrap();
        assert_eq!(url.unwrap(), "https://chat.deepseek.com/a/chat/s/x");
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0], msg("c"));
    }

    #[test]
    fn retries_last_message_when_retryable() {
        let mut store = SessionStore::new(20);
        let key = SessionStore::key("sys", "first");
        store.save(None, key.clone(), 2, Some("u".into()), true);
        let msgs = vec![msg("first"), msg("a")];
        let (_, _, rest) = store.find(&key, 1, &msgs).unwrap();
        assert_eq!(rest, vec![msg("a")]);
    }

    #[test]
    fn shorter_history_means_new_conversation() {
        let mut store = SessionStore::new(20);
        let key = SessionStore::key("sys", "first");
        store.save(None, key.clone(), 3, Some("u".into()), true);
        let msgs = vec![msg("first")];
        assert!(store.find(&key, 1, &msgs).is_none());
    }
}
