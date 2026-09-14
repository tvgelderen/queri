use super::*;

pub struct OpenGeneratedSql(pub crate::parameters::BoundQuery);
impl EventEmitter<OpenGeneratedSql> for Document {}

impl Document {
    pub(in crate::ui) fn seed_generated_parameters(
        &mut self,
        query: crate::parameters::BoundQuery,
    ) {
        use crate::parameters::{Definition, ParameterType};
        self.parameter_definitions = query
            .values
            .iter()
            .enumerate()
            .map(|(index, value)| Definition {
                name: (index + 1).to_string(),
                kind: if matches!(self.profile.config.dialect(), sql::SqlDialect::Postgres) {
                    ParameterType::Text
                } else {
                    ParameterType::for_cell(value)
                },
            })
            .collect();
        self.parameter_seed = Some(query);
    }

    pub(super) fn open_table_sql(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        let (Some(table), Some(options), Some(session)) = (
            self.table_ref().cloned(),
            self.applied_browse.clone(),
            self.session.clone(),
        ) else {
            self.error = "Load a table page successfully before opening its SQL.".into();
            cx.notify();
            return;
        };
        let generation = self.session_generation;
        self.operation = Operation::Running;
        self.error.clear();
        task(
            &self.services,
            window,
            cx,
            async move { session.browse_sql(table, options).await },
            move |this, result, _, cx| {
                if this.session_generation != generation {
                    return;
                }
                this.operation = Operation::Idle;
                match result {
                    Ok(query) => {
                        this.status = "Opened SQL for the loaded page; no query executed.".into();
                        cx.emit(OpenGeneratedSql(query));
                    }
                    Err(error) => {
                        this.error = format!(
                            "Could not open page SQL: {error:#}. Repair the table view and reload it."
                        )
                    }
                }
                cx.notify();
            },
        );
        cx.notify();
    }
}
