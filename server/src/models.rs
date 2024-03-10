use diesel::prelude::*;

#[derive(Queryable, Selectable)]
#[diesel(table_name = crate::schema::pipelines)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Pipeline {
    pub id: i32,
    pub packages: String,
    pub archs: String,
    pub git_branch: String,
    pub git_sha: String,
    pub creation_time: chrono::DateTime<chrono::Utc>,
}
