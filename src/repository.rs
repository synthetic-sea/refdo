use std::{collections::HashSet, path::PathBuf};

use gix::refs::Category;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BranchSection {
    pub full_ref_name: String,
    pub display_name: String,
    pub worktree_path: PathBuf,
    pub is_current: bool,
    pub is_locked: bool,
    pub is_stored_only: bool,
}

#[derive(Debug)]
pub struct RepositoryContext {
    pub head_label: String,
    pub common_git_dir: PathBuf,
    pub sections: Vec<BranchSection>,
}

impl Default for RepositoryContext {
    fn default() -> Self {
        Self {
            head_label: "unknown".to_owned(),
            common_git_dir: PathBuf::new(),
            sections: Vec::new(),
        }
    }
}

impl RepositoryContext {
    #[allow(clippy::result_large_err)]
    pub fn discover(path: impl AsRef<std::path::Path>) -> Result<Self, gix::discover::Error> {
        let repository = gix::discover(path)?;
        let current_git_dir = repository.git_dir().to_path_buf();
        let common_git_dir = repository
            .common_dir()
            .canonicalize()
            .unwrap_or_else(|_| repository.common_dir().to_path_buf());
        let head_label = head_label(&repository);
        let mut current = branch_section(&repository, &current_git_dir, false);
        let mut others = Vec::new();

        if let Ok(main_repository) = repository.main_repo()
            && main_repository.git_dir() != current_git_dir
            && let Some(section) = branch_section(&main_repository, &current_git_dir, false)
        {
            others.push(section);
        }

        if let Ok(worktrees) = repository.worktrees() {
            for worktree in worktrees {
                let is_locked = worktree.is_locked();
                let linked_repository = if is_locked {
                    worktree
                        .into_repo_with_possibly_inaccessible_worktree()
                        .ok()
                } else {
                    worktree.into_repo().ok()
                };
                let Some(linked_repository) = linked_repository else {
                    continue;
                };
                if linked_repository.git_dir() == current_git_dir {
                    continue;
                }
                if let Some(section) =
                    branch_section(&linked_repository, &current_git_dir, is_locked)
                {
                    others.push(section);
                }
            }
        }

        others.sort_by(|left, right| {
            left.display_name
                .cmp(&right.display_name)
                .then_with(|| left.worktree_path.cmp(&right.worktree_path))
        });

        let mut sections = Vec::with_capacity(usize::from(current.is_some()) + others.len());
        let mut seen = HashSet::with_capacity(sections.capacity());
        if let Some(section) = current.take() {
            seen.insert(section.full_ref_name.clone());
            sections.push(section);
        }
        for section in others {
            if seen.insert(section.full_ref_name.clone()) {
                sections.push(section);
            }
        }

        Ok(Self {
            head_label,
            common_git_dir,
            sections,
        })
    }

    pub fn reconcile_stored_branches<'a>(
        &mut self,
        branch_refs: impl IntoIterator<Item = &'a str>,
    ) {
        self.sections.retain(|section| !section.is_stored_only);
        let mut seen = self
            .sections
            .iter()
            .map(|section| section.full_ref_name.clone())
            .collect::<HashSet<_>>();
        let mut stored_only = branch_refs
            .into_iter()
            .filter(|branch_ref| seen.insert((*branch_ref).to_owned()))
            .map(|branch_ref| BranchSection {
                full_ref_name: branch_ref.to_owned(),
                display_name: branch_ref
                    .strip_prefix("refs/heads/")
                    .unwrap_or(branch_ref)
                    .to_owned(),
                worktree_path: PathBuf::new(),
                is_current: false,
                is_locked: false,
                is_stored_only: true,
            })
            .collect::<Vec<_>>();
        stored_only.sort_by(|left, right| {
            left.display_name
                .cmp(&right.display_name)
                .then_with(|| left.full_ref_name.cmp(&right.full_ref_name))
        });
        self.sections.extend(stored_only);
    }
}

fn branch_section(
    repository: &gix::Repository,
    current_git_dir: &std::path::Path,
    allow_inaccessible: bool,
) -> Option<BranchSection> {
    let worktree = repository.worktree()?;
    if !allow_inaccessible && !worktree.dot_git_exists() {
        return None;
    }

    let head = repository.head().ok()?;
    let branch = head.referent_name()?;
    if branch.category() != Some(Category::LocalBranch) {
        return None;
    }

    Some(BranchSection {
        full_ref_name: branch.to_string(),
        display_name: branch.shorten().to_string(),
        worktree_path: worktree.base().to_path_buf(),
        is_current: repository.git_dir() == current_git_dir,
        is_locked: worktree.is_locked(),
        is_stored_only: false,
    })
}

fn head_label(repository: &gix::Repository) -> String {
    let Ok(head) = repository.head() else {
        return "unknown".to_owned();
    };

    if let Some(branch) = head.referent_name() {
        return branch.shorten().to_string();
    }

    head.id()
        .map(|id| {
            let mut commit = id.to_string();
            commit.truncate(7);
            format!("detached@{commit}")
        })
        .unwrap_or_else(|| "unknown".to_owned())
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        process::Command,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use super::*;

    static FIXTURE_ID: AtomicUsize = AtomicUsize::new(0);

    struct Fixture {
        base: PathBuf,
        main: PathBuf,
        feature: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let id = FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
            let base =
                std::env::temp_dir().join(format!("refdo-worktrees-{}-{id}", std::process::id()));
            let main = base.join("main");
            let feature = base.join("feature");
            let detached = base.join("detached");
            let stale = base.join("stale");
            let offline = base.join("offline");

            fs::create_dir_all(&base).unwrap();
            git(&base, &["init", "-b", "main", main.to_str().unwrap()]);
            git(
                &main,
                &[
                    "-c",
                    "user.name=Refdo",
                    "-c",
                    "user.email=refdo@example.invalid",
                    "commit",
                    "--allow-empty",
                    "-m",
                    "initial",
                ],
            );
            git(
                &main,
                &[
                    "worktree",
                    "add",
                    "-b",
                    "feature/auth",
                    feature.to_str().unwrap(),
                ],
            );
            git(&main, &["worktree", "lock", feature.to_str().unwrap()]);
            git(
                &main,
                &[
                    "worktree",
                    "add",
                    "-b",
                    "offline",
                    offline.to_str().unwrap(),
                ],
            );
            git(&main, &["worktree", "lock", offline.to_str().unwrap()]);
            fs::remove_dir_all(offline).unwrap();
            git(
                &main,
                &["worktree", "add", "--detach", detached.to_str().unwrap()],
            );
            git(
                &main,
                &["worktree", "add", "-b", "stale", stale.to_str().unwrap()],
            );
            fs::remove_dir_all(stale).unwrap();

            Self {
                base,
                main,
                feature,
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.base);
        }
    }

    fn git(directory: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(directory)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn discovers_valid_worktree_branches_and_prioritizes_the_launch_worktree() {
        let fixture = Fixture::new();
        let expected_common_dir = fixture.main.join(".git").canonicalize().unwrap();

        let main_context = RepositoryContext::discover(&fixture.main).unwrap();
        assert_eq!(main_context.head_label, "main");
        assert_eq!(main_context.common_git_dir, expected_common_dir);
        assert_eq!(
            main_context
                .sections
                .iter()
                .map(|section| section.display_name.as_str())
                .collect::<Vec<_>>(),
            ["main", "feature/auth", "offline"]
        );
        assert!(main_context.sections[0].is_current);
        assert!(!main_context.sections[0].is_locked);
        assert!(!main_context.sections[1].is_current);
        assert!(main_context.sections[1].is_locked);
        assert!(!main_context.sections[2].is_current);
        assert!(main_context.sections[2].is_locked);

        let feature_context = RepositoryContext::discover(&fixture.feature).unwrap();
        assert_eq!(feature_context.head_label, "feature/auth");
        assert_eq!(
            feature_context
                .sections
                .iter()
                .map(|section| section.display_name.as_str())
                .collect::<Vec<_>>(),
            ["feature/auth", "main", "offline"]
        );
        assert!(feature_context.sections[0].is_current);
        assert!(feature_context.sections[0].is_locked);
        assert_eq!(feature_context.common_git_dir, expected_common_dir);
        assert!(
            feature_context
                .sections
                .iter()
                .all(|section| !section.is_stored_only)
        );
    }

    #[test]
    fn reconciles_stored_only_branches_without_duplicating_or_moving_worktrees() {
        let fixture = Fixture::new();
        let mut context = RepositoryContext::discover(&fixture.main).unwrap();

        context.reconcile_stored_branches([
            "refs/heads/z-stored",
            "refs/heads/main",
            "refs/heads/a-stored",
        ]);

        assert_eq!(
            context
                .sections
                .iter()
                .map(|section| section.display_name.as_str())
                .collect::<Vec<_>>(),
            ["main", "feature/auth", "offline", "a-stored", "z-stored"]
        );
        assert!(!context.sections[0].is_stored_only);
        assert!(context.sections[3].is_stored_only);
        assert!(context.sections[4].is_stored_only);
        assert!(context.sections[3].worktree_path.as_os_str().is_empty());

        context.reconcile_stored_branches(["refs/heads/z-stored"]);
        assert_eq!(
            context
                .sections
                .iter()
                .map(|section| section.display_name.as_str())
                .collect::<Vec<_>>(),
            ["main", "feature/auth", "offline", "z-stored"]
        );
        assert!(!context.sections[0].is_stored_only);
        assert!(context.sections[3].is_stored_only);

        context.reconcile_stored_branches([]);
        assert_eq!(
            context
                .sections
                .iter()
                .map(|section| section.display_name.as_str())
                .collect::<Vec<_>>(),
            ["main", "feature/auth", "offline"]
        );
        assert!(
            context
                .sections
                .iter()
                .all(|section| !section.is_stored_only)
        );
        assert!(context.sections[0].is_current);
    }

    #[test]
    fn discovers_runtime_worktree_additions_and_removals() {
        let fixture = Fixture::new();
        let dynamic = fixture.base.join("dynamic");
        let initial = RepositoryContext::discover(&fixture.main).unwrap();
        assert!(!initial.sections.iter().any(|s| s.display_name == "dynamic"));

        git(
            &fixture.main,
            &[
                "worktree",
                "add",
                "-b",
                "dynamic",
                dynamic.to_str().unwrap(),
            ],
        );
        let added = RepositoryContext::discover(&fixture.main).unwrap();
        assert!(
            added
                .sections
                .iter()
                .any(|s| s.display_name == "dynamic" && !s.is_stored_only)
        );

        git(
            &fixture.main,
            &["worktree", "remove", dynamic.to_str().unwrap()],
        );
        let removed = RepositoryContext::discover(&fixture.main).unwrap();
        assert!(!removed.sections.iter().any(|s| s.display_name == "dynamic"));
    }
    #[test]
    fn includes_the_current_unborn_branch() {
        let id = FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        let base = std::env::temp_dir().join(format!("refdo-unborn-{}-{id}", std::process::id()));
        let repository = base.join("repository");
        fs::create_dir_all(&base).unwrap();
        git(
            &base,
            &["init", "-b", "planned-work", repository.to_str().unwrap()],
        );

        let context = RepositoryContext::discover(&repository).unwrap();

        assert_eq!(context.head_label, "planned-work");
        assert_eq!(context.sections.len(), 1);
        assert_eq!(context.sections[0].full_ref_name, "refs/heads/planned-work");
        assert!(context.sections[0].is_current);
        fs::remove_dir_all(base).unwrap();
    }
}
