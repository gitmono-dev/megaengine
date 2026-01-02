use anyhow::Result;
use sea_orm::entity::prelude::*;
use sea_orm::{Set, Unchanged};

use crate::storage::init_db;

/// Refs table entity for tracking branch and tag commits
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "refs")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: String,
    pub repo_id: String,
    pub ref_name: String,
    pub commit_hash: String,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

/// Save or update a ref in the database
pub async fn save_ref(repo_id: &str, ref_name: &str, commit_hash: &str) -> Result<()> {
    let db = init_db().await?;
    let now = chrono::Local::now().timestamp();

    // Generate a unique ID for this ref record (repo_id + ref_name)
    let id = format!("{}:{}", repo_id, ref_name);

    // Check if ref already exists
    let existing = Entity::find_by_id(id.clone()).one(&db).await?;

    if let Some(_) = existing {
        // Update existing ref
        let active_model = ActiveModel {
            id: Unchanged(id),
            repo_id: Unchanged(repo_id.to_string()),
            ref_name: Unchanged(ref_name.to_string()),
            commit_hash: Set(commit_hash.to_string()),
            updated_at: Set(now),
        };
        Entity::update(active_model).exec(&db).await?;
    } else {
        // Insert new ref
        let active_model = ActiveModel {
            id: Set(id),
            repo_id: Set(repo_id.to_string()),
            ref_name: Set(ref_name.to_string()),
            commit_hash: Set(commit_hash.to_string()),
            updated_at: Set(now),
        };
        Entity::insert(active_model).exec(&db).await?;
    }

    Ok(())
}

/// Batch save multiple refs for a repository
pub async fn batch_save_refs(
    repo_id: &str,
    refs: &std::collections::HashMap<String, String>,
) -> Result<()> {
    let db = init_db().await?;
    let now = chrono::Local::now().timestamp();

    for (ref_name, commit_hash) in refs {
        let id = format!("{}:{}", repo_id, ref_name);

        let existing = Entity::find_by_id(id.clone()).one(&db).await?;

        if let Some(_) = existing {
            let active_model = ActiveModel {
                id: Unchanged(id),
                repo_id: Unchanged(repo_id.to_string()),
                ref_name: Unchanged(ref_name.clone()),
                commit_hash: Set(commit_hash.clone()),
                updated_at: Set(now),
            };
            Entity::update(active_model).exec(&db).await?;
        } else {
            let active_model = ActiveModel {
                id: Set(id),
                repo_id: Set(repo_id.to_string()),
                ref_name: Set(ref_name.clone()),
                commit_hash: Set(commit_hash.clone()),
                updated_at: Set(now),
            };
            Entity::insert(active_model).exec(&db).await?;
        }
    }

    Ok(())
}

/// Load all refs for a repository
pub async fn load_refs_for_repo(
    repo_id: &str,
) -> Result<std::collections::HashMap<String, String>> {
    let db = init_db().await?;

    let refs = Entity::find()
        .filter(Column::RepoId.eq(repo_id))
        .all(&db)
        .await?;

    let mut result = std::collections::HashMap::new();
    for ref_record in refs {
        result.insert(ref_record.ref_name, ref_record.commit_hash);
    }

    Ok(result)
}

/// Get a specific ref by repo_id and ref_name
pub async fn get_ref(repo_id: &str, ref_name: &str) -> Result<Option<String>> {
    let db = init_db().await?;
    let id = format!("{}:{}", repo_id, ref_name);

    if let Some(model) = Entity::find_by_id(id).one(&db).await? {
        return Ok(Some(model.commit_hash));
    }

    Ok(None)
}

/// Delete all refs for a repository
pub async fn delete_refs_for_repo(repo_id: &str) -> Result<()> {
    let db = init_db().await?;
    Entity::delete_many()
        .filter(Column::RepoId.eq(repo_id))
        .exec(&db)
        .await?;
    Ok(())
}

/// Delete a specific ref
pub async fn delete_ref(repo_id: &str, ref_name: &str) -> Result<()> {
    let db = init_db().await?;
    let id = format!("{}:{}", repo_id, ref_name);
    Entity::delete_by_id(id).exec(&db).await?;
    Ok(())
}

/// Check if any ref in the repository has been updated
pub async fn has_refs_changed(
    repo_id: &str,
    old_refs: &std::collections::HashMap<String, String>,
) -> Result<bool> {
    let db = init_db().await?;

    let current_refs = Entity::find()
        .filter(Column::RepoId.eq(repo_id))
        .all(&db)
        .await?;

    // If counts don't match, something has changed
    if current_refs.len() != old_refs.len() {
        return Ok(true);
    }

    // Check if any ref's commit hash has changed
    for ref_record in current_refs {
        if let Some(old_commit) = old_refs.get(&ref_record.ref_name) {
            if old_commit != &ref_record.commit_hash {
                return Ok(true);
            }
        } else {
            // New ref found
            return Ok(true);
        }
    }

    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_save_and_load_ref() -> Result<()> {
        let repo_id = "did:repo:test-ref-001";
        let ref_name = "refs/heads/main";
        let commit_hash = "abc123def456";

        // Save ref
        save_ref(repo_id, ref_name, commit_hash).await?;

        // Load ref
        let loaded = get_ref(repo_id, ref_name).await?;
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap(), commit_hash);

        // Cleanup
        delete_ref(repo_id, ref_name).await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_batch_save_refs() -> Result<()> {
        let repo_id = "did:repo:test-ref-002";
        let mut refs = std::collections::HashMap::new();
        refs.insert("refs/heads/main".to_string(), "abc123".to_string());
        refs.insert("refs/heads/develop".to_string(), "def456".to_string());
        refs.insert("refs/tags/v1.0".to_string(), "ghi789".to_string());

        // Save all refs
        batch_save_refs(repo_id, &refs).await?;

        // Load and verify
        let loaded = load_refs_for_repo(repo_id).await?;
        assert_eq!(loaded.len(), 3);
        assert_eq!(loaded.get("refs/heads/main"), Some(&"abc123".to_string()));
        assert_eq!(
            loaded.get("refs/heads/develop"),
            Some(&"def456".to_string())
        );
        assert_eq!(loaded.get("refs/tags/v1.0"), Some(&"ghi789".to_string()));

        // Cleanup
        delete_refs_for_repo(repo_id).await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_has_refs_changed() -> Result<()> {
        let repo_id = "did:repo:test-ref-003";
        let mut refs = std::collections::HashMap::new();
        refs.insert("refs/heads/main".to_string(), "abc123".to_string());

        // Save initial refs
        batch_save_refs(repo_id, &refs).await?;

        // No change
        let changed = has_refs_changed(repo_id, &refs).await?;
        assert!(!changed);

        // Change commit hash
        refs.insert("refs/heads/main".to_string(), "def456".to_string());
        let changed = has_refs_changed(repo_id, &refs).await?;
        assert!(changed);

        // Cleanup
        delete_refs_for_repo(repo_id).await?;
        Ok(())
    }
}
