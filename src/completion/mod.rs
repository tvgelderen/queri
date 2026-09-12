//! SQL completion analysis is independent of the editor and database transport.
pub mod analysis;

#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub relations: Vec<Relation>,
    pub schemas: Vec<String>,
    pub status: String,
}

#[derive(Clone, Debug)]
pub struct Relation {
    pub schema: String,
    pub name: String,
    pub visible: bool,
    pub columns: Option<Vec<Column>>,
}

#[derive(Clone, Debug)]
pub struct Column {
    pub name: String,
    pub detail: String,
}
