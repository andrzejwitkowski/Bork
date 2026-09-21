use bork::lsp::{dump_arenas_for_source, hover_for_source, diagnostics_for_source};
use std::collections::HashMap;
use std::sync::Mutex;
use tower_lsp::jsonrpc::{Error, Result};
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

const DUMP_ARENAS_CMD: &str = "bork.dumpArenas";

#[derive(Debug)]
struct Backend {
    client: Client,
    docs: Mutex<HashMap<Url, String>>,
}

impl Backend {
    fn doc(&self, uri: &Url) -> Result<Option<String>> {
        Ok(self
            .docs
            .lock()
            .map_err(|_| Error::internal_error())?
            .get(uri)
            .cloned())
    }

    async fn publish_for(&self, uri: Url, text: &str) {
        {
            let mut docs = self.docs.lock().expect("docs mutex");
            docs.insert(uri.clone(), text.to_string());
        }
        self.client
            .publish_diagnostics(uri, diagnostics_for_source(text), None)
            .await;
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                execute_command_provider: Some(ExecuteCommandOptions {
                    commands: vec![DUMP_ARENAS_CMD.into()],
                    ..Default::default()
                }),
                ..ServerCapabilities::default()
            },
            ..InitializeResult::default()
        })
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        self.publish_for(params.text_document.uri, &params.text_document.text)
            .await;
    }

    async fn did_change(&self, mut params: DidChangeTextDocumentParams) {
        let Some(change) = params.content_changes.pop() else {
            return;
        };
        self.publish_for(params.text_document.uri, &change.text)
            .await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        if let Ok(mut docs) = self.docs.lock() {
            docs.remove(&params.text_document.uri);
        }
        self.client
            .publish_diagnostics(params.text_document.uri, Vec::new(), None)
            .await;
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;
        let Some(text) = self.doc(uri)? else {
            return Ok(None);
        };
        Ok(hover_for_source(&text, pos).map(|value| Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value,
            }),
            range: None,
        }))
    }

    async fn execute_command(
        &self,
        params: ExecuteCommandParams,
    ) -> Result<Option<serde_json::Value>> {
        if params.command != DUMP_ARENAS_CMD {
            return Err(Error::method_not_found());
        }
        let text = match params
            .arguments
            .first()
            .and_then(|v| v.as_str())
            .and_then(|s| Url::parse(s).ok())
        {
            Some(uri) => self.doc(&uri)?,
            None => {
                let docs = self.docs.lock().map_err(|_| Error::internal_error())?;
                docs.values().next().cloned()
            }
        };
        let Some(text) = text else {
            return Ok(Some(serde_json::json!({
                "error": "no open Bork document"
            })));
        };
        match dump_arenas_for_source(&text) {
            Ok(dump) => {
                self.client
                    .log_message(MessageType::INFO, dump.clone())
                    .await;
                Ok(Some(serde_json::json!({ "dump": dump })))
            }
            Err(err) => Ok(Some(serde_json::json!({ "error": err }))),
        }
    }
}

#[tokio::main]
async fn main() {
    let (service, socket) = LspService::new(|client| Backend {
        client,
        docs: Mutex::new(HashMap::new()),
    });
    Server::new(tokio::io::stdin(), tokio::io::stdout(), socket)
        .serve(service)
        .await;
}
