use bincode::{self, Decode, Encode};
use std::{
    collections::HashSet,
    mem,
    path::{Path, PathBuf},
};

// https://www.ams.org/journals/mcom/1999-68-225/S0025-5718-99-00996-5/S0025-5718-99-00996-5.pdf
// not using an rng from the rand crate makes (de)serialization easier
const RNG_A: usize = 35;
const RNG_MOD: usize = 509;

#[derive(Clone, Debug, Decode, Encode, PartialEq)]
pub struct Entry {
    pub id: u32,
    pub path: PathBuf,
}

#[derive(Clone, Debug, Decode, Encode)]
struct Rng(usize);

#[derive(Clone, Debug, Decode, Encode)]
struct Random {
    rng: Rng,
    ids: Vec<u32>,
}

#[derive(Clone, Debug, Decode, Default, Encode)]
enum QueueMode {
    #[default]
    Sequential,
    Single,
    Random(Random),
}

#[derive(Clone, Debug, Decode, Default, Encode)]
pub struct Queue {
    list: Vec<Entry>,
    pos: Option<usize>,
    mode: QueueMode,
    history: HashSet<u32>,
    next_id: u32,
}

impl From<(u32, PathBuf)> for Entry {
    fn from((id, path): (u32, PathBuf)) -> Self {
        Self { id, path }
    }
}

impl Rng {
    pub fn next_usize(&mut self, l: usize, r: usize) -> usize {
        self.0 = (self.0 * RNG_A) % RNG_MOD;
        self.0 % (r - l + 1) + l
    }
}

impl Random {
    pub fn new(mut ids: Vec<u32>) -> Self {
        let mut rng = Rng(ids.len());
        // Fisher-Yates shuffle
        for i in 0..(ids.len().saturating_sub(1)) {
            let j = rng.next_usize(i, ids.len().saturating_sub(1));
            ids.swap(i, j);
        }

        Self { rng, ids }
    }
}

impl Queue {
    pub fn find_by_id(&self, id: u32) -> Option<usize> {
        self.list.iter().position(|entry| entry.id == id)
    }

    pub fn mode(&self) -> String {
        match self.mode {
            QueueMode::Sequential => "sequential",
            QueueMode::Single => "single",
            QueueMode::Random(_) => "random",
        }
        .into()
    }

    pub fn current(&self) -> Option<&Entry> {
        self.pos.map(|pos| &self.list[pos])
    }

    pub fn inner(&self) -> &[Entry] {
        &self.list
    }

    pub fn reset_pos(&mut self) {
        let _ = self.pos.take();
    }

    pub fn add_current_to_history(&mut self) {
        if let Some(current) = self.current() {
            self.history.insert(current.id);
        }
    }

    pub fn move_next(&mut self) -> Option<&Entry> {
        match &mut self.mode {
            QueueMode::Sequential => match &mut self.pos {
                Some(pos) if *pos < self.list.len().saturating_sub(1) => *pos += 1,
                None if !self.list.is_empty() => self.pos = Some(0),
                _ => self.pos = None,
            },
            QueueMode::Single => {
                let _ = self.pos.take();
            }
            QueueMode::Random(random) => match random.ids.pop() {
                Some(id) => self.pos = self.find_by_id(id),
                None => {
                    // random pool exhausted
                    let ids: Vec<_> = self.list.iter().map(|entry| entry.id).collect();
                    if ids.is_empty() {
                        self.pos = None;
                    } else {
                        self.mode = QueueMode::Random(Random::new(ids));
                        // this won't recurse more because
                        // the Some(id) branch will be taken
                        self.move_next();
                    }
                }
            },
        }

        self.current()
    }

    pub fn move_prev(&mut self) -> Option<&Entry> {
        match &mut self.pos {
            Some(pos) if *pos > 0 => *pos -= 1,
            None if !self.list.is_empty() => self.pos = Some(self.list.len().saturating_sub(1)),
            _ => self.pos = None,
        };

        self.current()
    }

    pub fn move_to(&mut self, id: u32) -> Option<&Entry> {
        // without this check, you could manually play song X and then
        // still get song X from the random pool later
        if let QueueMode::Random(Random { rng: _, ids }) = &mut self.mode {
            ids.retain(|&r_id| r_id != id);
        };

        if let Some(pos) = self.find_by_id(id) {
            self.pos = Some(pos);
            self.current()
        } else {
            None
        }
    }

    pub fn add(&mut self, path: impl AsRef<Path> + Into<PathBuf>, pos: Option<usize>) {
        self.next_id += 1;
        let id = self.next_id;
        let entry = Entry {
            id,
            path: path.into(),
        };

        match pos {
            Some(pos) if pos < self.list.len() => self.list.insert(pos, entry),
            _ => self.list.push(entry),
        }
        if let QueueMode::Random(Random { rng, ids }) = &mut self.mode {
            if ids.is_empty() {
                ids.push(id);
            } else {
                // add to a random position in constant time
                let random_pos = rng.next_usize(0, ids.len().saturating_sub(1));
                let temp = mem::replace(&mut ids[random_pos], id);
                ids.push(temp);
            }
        }
    }

    // does nothing if the id is invalid
    // returns true if the currently playing song was removed
    pub fn remove(&mut self, id: u32) -> bool {
        if let QueueMode::Random(Random { rng: _, ids }) = &mut self.mode {
            ids.retain(|&r_id| r_id != id);
        };
        if let Some(removed_pos) = self.find_by_id(id) {
            self.list.remove(removed_pos);
            if let Some(cur_pos) = self.pos {
                if cur_pos == removed_pos {
                    self.pos = None;
                    return true;
                } else {
                    if cur_pos > removed_pos {
                        self.pos = Some(cur_pos.saturating_sub(1));
                    }
                    return false;
                }
            }
        }

        false
    }

    pub fn clear(&mut self) {
        self.list.clear();
        self.history.clear();
        let _ = self.pos.take();
        self.next_id = 0;
    }

    pub fn start_random(&mut self) {
        let mut not_played_ids: Vec<_> = self
            .list
            .iter()
            .filter(|entry| {
                !self.history.contains(&entry.id)
                    && self
                        .current()
                        .map(|cur_entry| entry.id != cur_entry.id)
                        .unwrap_or(true)
            })
            .map(|entry| entry.id)
            .collect();
        // if we've already played every song, start again
        if not_played_ids.is_empty() {
            not_played_ids = self.list.iter().map(|entry| entry.id).collect();
        }
        self.mode = QueueMode::Random(Random::new(not_played_ids));
    }

    pub fn start_sequential(&mut self) {
        self.mode = QueueMode::Sequential;
    }

    pub fn start_single(&mut self) {
        self.mode = QueueMode::Single;
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn add_and_remove() {
        let mut queue = Queue::default();
        queue.add("a", None);
        queue.add("b", Some(0));
        queue.add("c", None);
        queue.add("d", Some(2));
        let expected = &[
            (2, "b".into()).into(),
            (1, "a".into()).into(),
            (4, "d".into()).into(),
            (3, "c".into()).into(),
        ];
        assert_eq!(queue.inner(), expected);

        queue.remove(4);
        queue.remove(2137);
        queue.add("e", None);
        queue.remove(2);
        queue.add("f", Some(1));
        let expected = &[
            (1, "a".into()).into(),
            (6, "f".into()).into(),
            (3, "c".into()).into(),
            (5, "e".into()).into(),
        ];
        assert_eq!(queue.inner(), expected);
    }

    #[test]
    fn traversing() {
        let mut queue = Queue::default();
        let n = 5;
        for i in 1..=n {
            queue.add(format!("song{}", i), None);
        }

        queue.move_next();
        queue.move_next();
        assert_eq!(queue.current(), Some((2, "song2".into()).into()).as_ref());
        queue.move_next();
        queue.move_prev();
        assert_eq!(queue.current(), Some((2, "song2".into()).into()).as_ref());
        queue.move_next();
        queue.move_next();
        assert_eq!(queue.current(), Some((4, "song4".into()).into()).as_ref());
        queue.move_next();
        queue.move_next();
        assert_eq!(queue.current(), None);
        queue.move_prev();
        assert_eq!(queue.current(), Some((5, "song5".into()).into()).as_ref());
        queue.move_next();
        queue.move_next();
        assert_eq!(queue.current(), Some((1, "song1".into()).into()).as_ref());
    }
}
