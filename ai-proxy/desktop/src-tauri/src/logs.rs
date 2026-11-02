use std::collections::VecDeque;
use std::sync::Mutex;

pub struct LogBuffer {
    cap: usize,
    lines: Mutex<VecDeque<String>>,
}

impl LogBuffer {
    pub fn new(cap: usize) -> Self {
        Self {
            cap,
            lines: Mutex::new(VecDeque::new()),
        }
    }

    pub fn push(&self, msg: impl Into<String>) {
        let line = format!(
            "[{}] {}",
            chrono::Local::now().format("%H:%M:%S"),
            msg.into()
        );
        let mut lines = self.lines.lock().unwrap();
        lines.push_back(line);
        while lines.len() > self.cap {
            lines.pop_front();
        }
    }

    pub fn snapshot(&self) -> Vec<String> {
        self.lines.lock().unwrap().iter().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_only_the_newest() {
        let b = LogBuffer::new(3);
        for i in 0..5 {
            b.push(format!("msg{i}"));
        }
        let snap = b.snapshot();
        assert_eq!(snap.len(), 3);
        assert!(snap[0].ends_with("msg2"));
        assert!(snap[2].ends_with("msg4"));
    }
}
