use bork::lsp::{analyze_source, dump_from_analysis, hover_for_analysis, Analysis};
use std::collections::HashMap;
use std::sync::Mutex;
use tower_lsp::jsonrpc::{Error, Result};
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

const DUMP_ARENAS_CMD: &str = "bork.dumpArenas";

struct DocState {
    text: String,
    analysis: Analysis,
}

struct Backend {
    client: Client,
    docs: Mutex<HashMap<Url, DocState>>,
    /// Bumped on every open/change/close so in-flight analyses cannot commit stale state.
    epoch: Mutex<HashMap<Url, u64>>,
}

impl Backend {
    fn bump_epoch(&self, uri: &Url) -> u64 {
        let mut epochs = self.epoch.lock().expect("epoch mutex");
        let slot = epochs.entry(uri.clone()).or_insert(0);
        *slot += 1;
        *slot
    }

    async fn publish_for(&self, uri: Url, text: String, version: i32) {
        let epoch = self.bump_epoch(&uri);
        let analysis = analyze_source(&text);
        let diagnostics = analysis.diagnostics().to_vec();
        {
            let epochs = self.epoch.lock().expect("epoch mutex");
            if epochs.get(&uri).copied() != Some(epoch) {
                return;
            }
            let mut docs = self.docs.lock().expect("docs mutex");
            docs.insert(uri.clone(), DocState { text, analysis });
        }
        self.client
            .publish_diagnostics(uri, diagnostics, Some(version))
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
        let doc = params.text_document;
        self.publish_for(doc.uri, doc.text, doc.version).await;
    }

    async fn did_change(&self, mut params: DidChangeTextDocumentParams) {
        let Some(change) = params.content_changes.pop() else {
            return;
        };
        self.publish_for(
            params.text_document.uri,
            change.text,
            params.text_document.version,
        )
        .await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        self.bump_epoch(&uri);
        if let Ok(mut docs) = self.docs.lock() {
            docs.remove(&uri);
        }
        self.client
            .publish_diagnostics(uri, Vec::new(), None)
            .await;
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;
        let docs = self.docs.lock().map_err(|_| Error::internal_error())?;
        let Some(doc) = docs.get(uri) else {
            return Ok(None);
        };
        let value = doc
            .analysis
            .report()
            .and_then(|report| hover_for_analysis(report, &doc.text, pos));
        Ok(value.map(|value| Hover {
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
        let dump_result = match self.docs.lock() {
            Err(_) => return Err(Error::internal_error()),
            Ok(docs) => {
                let doc = match params
                    .arguments
                    .first()
                    .and_then(|v| v.as_str())
                    .and_then(|s| Url::parse(s).ok())
                {
                    Some(uri) => docs.get(&uri),
                    None => docs.values().next(),
                };
                doc.map(|doc| dump_from_analysis(&doc.analysis))
            }
        };
        let Some(result) = dump_result else {
            return Ok(Some(serde_json::json!({
                "error": "no open Bork document"
            })));
        };
        match result {
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
        epoch: Mutex::new(HashMap::new()),
    });
    Server::new(tokio::io::stdin(), tokio::io::stdout(), socket)
        .serve(service)
        .await;
}
