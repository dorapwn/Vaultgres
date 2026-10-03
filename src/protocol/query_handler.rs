//! Query handling
//!
//! Main entry point for handling client queries and managing
//! the PostgreSQL protocol connection.

use super::ddl_handler;
use super::dml_handler::{self, DmlResult};
use super::message::{Message, ProtocolError, Response};
use super::result_set::ResultSet;
use crate::catalog::Catalog;
use crate::parser::Parser;
use std::io::{Read, Write};
use std::sync::Arc;

pub enum ExecutionResult {
    CommandComplete(String),
    ResultSet(ResultSet),
}

pub struct Connection<S: Read + Write> {
    stream: S,
    authenticated: bool,
    catalog: Arc<Catalog>,
}

impl<S: Read + Write> Connection<S> {
    pub fn new(stream: S, catalog: Arc<Catalog>) -> Self {
        Self { stream, authenticated: false, catalog }
    }

    pub fn handle_startup(&mut self) -> Result<(), ProtocolError> {
        let mut len_buf = [0u8; 4];
        self.stream.read_exact(&mut len_buf)?;
        let len = i32::from_be_bytes(len_buf) as usize;

        let mut data = vec![0u8; len - 4];
        self.stream.read_exact(&mut data)?;

        let msg = Message::parse(0, &data)?;
        log::debug!("Startup message: {:?}", msg);
        self.authenticated = true;
        self.send_startup_response()?;
        Ok(())
    }

    /// Send the standard post-startup response: AuthenticationOk,
    /// session ParameterStatus, BackendKeyData, ReadyForQuery.
    ///
    /// Single source of truth for the wire-format sequence; called from
    /// both the SSL-negotiated path (after `N`) and the plain startup
    /// path (after the client sent the StartupMessage directly).
    ///
    /// Tracked by https://github.com/neoalienson/Vaultgres/issues/20
    fn send_startup_response(&mut self) -> Result<(), ProtocolError> {
        // PostgreSQL clients (psycopg2/3, asyncpg, psql, JDBC) expect:
        //   1. AuthenticationOk (R)
        //   2. Any number of ParameterStatus (S) and NoticeResponse (N)
        //   3. BackendKeyData (K) — needed for cancellation, pid of session
        //   4. ReadyForQuery (Z)
        //
        // Sending these in the wrong order, or omitting them, breaks
        // session-parameter parsing in psycopg3 in particular (the
        // DataRow decoder crashes when the protocol state machine is
        // misaligned by the missing ParameterStatus / BackendKeyData
        // sequence).
        Response::AuthenticationOk.write(&mut self.stream)?;

        for (name, value) in [
            ("client_encoding", "UTF8"),
            ("DateStyle", "ISO, MDY"),
            ("TimeZone", "UTC"),
            ("server_version", "16.0 (Vaultgres 0.1.0)"),
            ("server_encoding", "UTF8"),
        ] {
            Response::ParameterStatus { name: name.to_string(), value: value.to_string() }
                .write(&mut self.stream)?;
        }
        Response::BackendKeyData { pid: std::process::id() as i32, secret: 0 }
            .write(&mut self.stream)?;

        Response::ReadyForQuery.write(&mut self.stream)?;
        self.stream.flush()?;
        Ok(())
    }

    pub fn handle_query(&mut self, sql: &str) -> Result<(), ProtocolError> {
        log::info!("Query: {}", sql);
        match Parser::new(sql) {
            Ok(mut parser) => match parser.parse() {
                Ok(stmt) => {
                    log::debug!("Parsed statement: {:?}", stmt);
                    match self.execute_statement(stmt) {
                        Ok(ExecutionResult::CommandComplete(tag)) => {
                            Response::CommandComplete { tag }.write(&mut self.stream)?;
                            Response::ReadyForQuery.write(&mut self.stream)?;
                        }
                        Ok(ExecutionResult::ResultSet(result_set)) => {
                            Response::RowDescriptionDetailed {
                                columns: result_set.columns.clone(),
                            }
                            .write(&mut self.stream)?;

                            for row in &result_set.rows {
                                Response::DataRowDetailed { fields: row.fields.clone() }
                                    .write(&mut self.stream)?;
                            }

                            Response::CommandComplete {
                                tag: format!("SELECT {}", result_set.row_count()),
                            }
                            .write(&mut self.stream)?;
                            Response::ReadyForQuery.write(&mut self.stream)?;
                        }
                        Err(e) => {
                            log::warn!("Execution error: {}", e);
                            Response::ErrorResponse { message: format!("Execution error: {}", e) }
                                .write(&mut self.stream)?;
                            Response::ReadyForQuery.write(&mut self.stream)?;
                        }
                    }
                }
                Err(e) => {
                    log::warn!("Parse error: {}", e);
                    Response::ErrorResponse { message: format!("Parse error: {}", e) }
                        .write(&mut self.stream)?;
                    Response::ReadyForQuery.write(&mut self.stream)?;
                }
            },
            Err(e) => {
                log::warn!("Lexer error: {}", e);
                Response::ErrorResponse { message: format!("Lexer error: {}", e) }
                    .write(&mut self.stream)?;
                Response::ReadyForQuery.write(&mut self.stream)?;
            }
        }
        self.stream.flush()?;
        Ok(())
    }

    fn execute_statement(
        &self,
        stmt: crate::parser::ast::Statement,
    ) -> Result<ExecutionResult, String> {
        match &stmt {
            crate::parser::ast::Statement::Select(_)
            | crate::parser::ast::Statement::With(_)
            | crate::parser::ast::Statement::Insert(_)
            | crate::parser::ast::Statement::Update(_)
            | crate::parser::ast::Statement::Delete(_)
            | crate::parser::ast::Statement::Describe(_) => {
                match dml_handler::execute_dml(self.catalog.clone(), stmt) {
                    Ok(DmlResult::CommandComplete(tag)) => {
                        Ok(ExecutionResult::CommandComplete(tag))
                    }
                    Ok(DmlResult::ResultSet(rs)) => Ok(ExecutionResult::ResultSet(rs)),
                    Err(e) => Err(e),
                }
            }
            _ => match ddl_handler::execute_ddl(&self.catalog, stmt) {
                Ok(ddl_handler::DdlResult::CommandComplete(tag)) => {
                    Ok(ExecutionResult::CommandComplete(tag))
                }
                Err(e) => Err(e),
            },
        }
    }

    pub fn run(&mut self) -> Result<(), ProtocolError> {
        let mut first_bytes = [0u8; 8];
        if self.stream.read_exact(&mut first_bytes).is_err() {
            return Ok(());
        }

        let len =
            i32::from_be_bytes([first_bytes[0], first_bytes[1], first_bytes[2], first_bytes[3]]);
        let code =
            i32::from_be_bytes([first_bytes[4], first_bytes[5], first_bytes[6], first_bytes[7]]);

        if len == 8 && code == 80877103 {
            log::debug!("SSL negotiation rejected");
            self.stream.write_all(b"N")?;
            self.stream.flush()?;
            self.handle_startup()?;
        } else {
            // Not an SSLRequest — must be a startup message. Read the
            // rest of the payload, mark authenticated, then send the
            // canonical startup response (single source of truth in
            // send_startup_response). Previously this branch inlined
            // AuthenticationOk + ReadyForQuery only, which broke
            // psycopg3 / asyncpg session-parameter parsing.
            //
            // Tracked by https://github.com/neoalienson/Vaultgres/issues/20
            let mut remaining_data = vec![0u8; (len - 8) as usize];
            self.stream.read_exact(&mut remaining_data)?;
            let mut data = first_bytes[4..].to_vec();
            data.extend_from_slice(&remaining_data);
            let msg = Message::parse(0, &data)?;
            log::debug!("Startup message (non-SSL path): {:?}", msg);
            self.authenticated = true;
            self.send_startup_response()?;
        }

        loop {
            let mut tag_buf = [0u8; 1];
            if self.stream.read_exact(&mut tag_buf).is_err() {
                break;
            }

            let mut len_buf = [0u8; 4];
            self.stream.read_exact(&mut len_buf)?;
            let len = i32::from_be_bytes(len_buf) as usize;

            let mut data = vec![0u8; len - 4];
            self.stream.read_exact(&mut data)?;

            let msg = Message::parse(tag_buf[0], &data)?;

            match msg {
                Message::Query { sql } => self.handle_query(&sql)?,
                Message::Terminate => break,
                _ => {}
            }
        }
        Ok(())
    }
}

#[cfg(test)]
impl Connection<std::io::Cursor<Vec<u8>>> {
    pub fn dummy() -> Self {
        let catalog = Arc::new(Catalog::new());
        let cursor = std::io::Cursor::new(Vec::new());
        Connection::new(cursor, catalog)
    }
}
