use rand::{prelude::*, seq::SliceRandom};
use std::{
    collections::HashSet,
    mem,
    path::{Path, PathBuf},
    rc::Rc,
};

#[derive(Clone)]
pub struct Entry {
    pub id: u32,
    pub path: Rc<Path>, // absolute paths
}

#[derive(Debug)]
struct Random {
    rng: SmallRng,
    ids: Vec<u32>,
}

#[derive(Debug, Default)]
enum QueueMode {
    #[default]
    Sequential,
    Single,
    Random(Random),
}

#[derive(Default)]
pub struct Queue {
    list: Vec<Entry>,
    pos: Option<usize>,
    mode: QueueMode,
    history: HashSet<u32>,
    next_id: u32,
}

impl Random {
    pub fn new(mut ids: Vec<u32>) -> Self {
        let mut rng = SmallRng::from_os_rng();
        ids.shuffle(&mut rng);

        Self { rng, ids }
    }
}

impl Queue {
    fn find_by_id(&self, id: u32) -> Option<usize> {
        self.list.iter().position(|entry| entry.id == id)
    }

    pub fn new() -> Self {
        Self::default()
    }

    pub fn mode(&self) -> String {
        match self.mode {
            QueueMode::Sequential => "sequential",
            QueueMode::Single => "single",
            QueueMode::Random(_) => "random",
        }
        .into()
    }

    pub fn current(&self) -> Option<Entry> {
        self.pos.map(|pos| &self.list[pos]).cloned()
    }

    pub fn as_inner(&self) -> &[Entry] {
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

    pub fn move_next(&mut self) -> Option<Entry> {
        match &mut self.mode {
            QueueMode::Sequential => match &mut self.pos {
                Some(pos) if *pos < self.list.len() - 1 => *pos += 1,
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

    pub fn move_prev(&mut self) -> Option<Entry> {
        match &mut self.pos {
            Some(pos) if *pos > 0 => *pos -= 1,
            None if !self.list.is_empty() => self.pos = Some(self.list.len() - 1),
            _ => self.pos = None,
        };

        self.current()
    }

    pub fn move_to(&mut self, id: u32) -> Option<Entry> {
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

    pub fn add(&mut self, path: impl AsRef<Path>, pos: Option<usize>) {
        self.next_id += 1;
        let id = self.next_id;
        let entry = Entry {
            id,
            path: path.as_ref().into(),
        };

        match pos {
            Some(pos) if pos <= self.list.len() => self.list.insert(pos, entry),
            _ => self.list.push(entry),
        }
        if let QueueMode::Random(Random { rng, ids }) = &mut self.mode {
            if ids.is_empty() {
                ids.push(id);
            } else {
                // add to a random position in constant time
                let random_pos = rng.random_range(0..ids.len());
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
                        self.pos = Some(cur_pos - 1);
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
        let not_played_ids: Vec<_> = self
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
        self.mode = QueueMode::Random(Random::new(not_played_ids));
    }

    pub fn start_sequential(&mut self) {
        self.mode = QueueMode::Sequential;
    }

    pub fn start_single(&mut self) {
        self.mode = QueueMode::Single;
    }
}

/*
#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn add_and_remove() {
        let mut queue = Queue::new();
        queue.add(1001, None);
        queue.add(1002, Some(0));
        queue.add(1003, None);
        queue.add(1004, Some(2));
        let expected = &[
            (2, 1002).into(),
            (1, 1001).into(),
            (4, 1004).into(),
            (3, 1003).into(),
        ];
        assert_eq!(queue.as_inner(), expected);

        queue.remove(4);
        queue.remove(2137);
        queue.add(1005, None);
        queue.remove(2);
        queue.add(1006, Some(1));
        let expected = &[
            (1, 1001).into(),
            (6, 1006).into(),
            (3, 1003).into(),
            (5, 1005).into(),
        ];
        assert_eq!(queue.as_inner(), expected);
    }

    #[test]
    fn traversing() {
        let mut queue = Queue::new();
        let n = 5;
        for i in 1001..=(1000 + n) {
            queue.add(i, None);
        }

        queue.move_next();
        queue.move_next();
        assert_eq!(queue.current(), Some((2, 1002).into()));
        queue.move_next();
        queue.move_prev();
        assert_eq!(queue.current(), Some((2, 1002).into()));
        queue.move_next();
        queue.move_next();
        assert_eq!(queue.current(), Some((4, 1004).into()));
        queue.move_next();
        queue.move_next();
        assert_eq!(queue.current(), None);
        queue.move_prev();
        assert_eq!(queue.current(), Some((5, 1005).into()));
        queue.move_next();
        queue.move_next();
        assert_eq!(queue.current(), Some((1, 1001).into()));
    }

    #[test]
    fn random() {
        let mut queue = Queue::new();
        let n = 100;
        for i in 1000..(1000 + n) {
            queue.add(i, None);
        }

        let mut seen = HashSet::new();
        queue.move_next();
        let cur_on_toggle = queue.current();
        seen.insert(cur_on_toggle.unwrap().id);
        queue.start_random();
        for i in 0..(n - 1) {
            let cur = queue.current();
            if i == 0 {
                // check that toggling random doesn't "move" the current song
                assert_eq!(cur, cur_on_toggle);
            } else {
                assert!(cur.is_some() && !seen.contains(&cur.unwrap().id));
            }
            seen.insert(cur.unwrap().id);
            queue.move_next();
        }
    }
}
*/
