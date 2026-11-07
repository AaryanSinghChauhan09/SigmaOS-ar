//! Ar model — create/modify/list archives.

#[derive(Debug, Clone)]
pub struct Member {
    pub name: String,
    pub size: usize,
    pub offset: usize,
}

#[derive(Debug, Clone)]
pub struct Archive {
    pub members: Vec<Member>,
    pub thin: bool,
    pub deterministic: bool,
}

#[derive(Debug, Clone)]
pub struct Ar {
    pub verbose: bool,
    pub plugin: bool,
}

impl Default for Ar {
    fn default() -> Self { Self { verbose: false, plugin: false } }
}

impl Ar {
    pub fn new() -> Self { Self::default() }

    pub fn create(&self) -> Archive {
        Archive { members: vec![], thin: false, deterministic: true }
    }

    pub fn add(&self, archive: &mut Archive, name: &str, size: usize) {
        let offset = archive.members.iter().map(|m| m.size).sum::<usize>() + archive.members.len() * 60;
        archive.members.push(Member { name: name.to_string(), size, offset });
    }

    pub fn list(&self, archive: &Archive) -> Vec<String> {
        archive.members.iter().map(|m| m.name.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn add_list() {
        let a = Ar::new();
        let mut arc = a.create();
        a.add(&mut arc, "foo.o", 100);
        assert_eq!(a.list(&arc), vec!["foo.o"]);
    }
}
