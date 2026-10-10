# Dendro — Guia de Extensão de Conectores (Extension Guide)

O Dendro foi arquitetado para ser desacoplado e extensível. Se a sua equipe utiliza um harness de desenvolvimento agêntico novo (como Cursor, Windsurf, Devin, Goose, Aider ou frameworks proprietários), este guia detalha como criar um conector e registrá-lo no motor analítico.

---

## 1. O Contrato Canônico: `HarnessConnector`

Todo novo conector deve implementar a trait `HarnessConnector` definida em `crates/dendro-core/src/traits.rs`:

```rust
use async_trait::async_trait;
use crate::models::{CanonicalSession, Watermark};
use crate::error::DendroResult;
use std::path::Path;

#[async_trait]
pub trait HarnessConnector: Send + Sync {
    /// Identificador canônico do conector (ex: "cursor", "windsurf", "devin")
    fn name(&self) -> &'static str;

    /// Verifica se o ambiente local possui dados deste conector
    async fn is_available(&self) -> bool;

    /// Descobre caminhos de sessões locais para ingestão
    async fn discover_sessions(&self) -> DendroResult<Vec<std::path::PathBuf>>;

    /// Faz a extração e conversão de uma sessão a partir do watermark fornecido
    async fn extract_session(
        &self,
        session_path: &Path,
        watermark: Option<&Watermark>,
    ) -> DendroResult<CanonicalSession>;
}
```

---

## 2. O Modelo Canônico: `CanonicalSession`

O objetivo do seu conector é converter os logs heterogêneos do seu harness no schema canônico universal:

| Campo | Tipo | Descrição |
| :--- | :--- | :--- |
| `id` | `String` | Identificador único estável da sessão (UUID ou hash do path) |
| `harness` | `String` | Nome do harness (ex: `"cursor"`) |
| `agent_name` | `Option<String>` | Assento ou identificador declarado do agente |
| `rig_id` | `Option<String>` | ID do rig (se resolvido pelo conector) |
| `cwd` | `Option<String>` | Diretório de trabalho da sessão |
| `started_at` | `DateTime<Utc>` | Timestamp de início |
| `ended_at` | `Option<DateTime<Utc>>`| Timestamp de encerramento |
| `input_tokens` | `u64` | Total de tokens de prompt consumidos |
| `output_tokens`| `u64` | Total de tokens gerados |
| `cache_read_tokens` | `u64` | Tokens lidos de cache (quando suportado pelo LLM) |
| `tool_calls` | `Vec<CanonicalToolCall>`| Chamadas de ferramentas executadas durante a sessão |
| `skills_invoked`| `Vec<String>` | Nomes de skills/comandos especializados acionados |

---

## 3. Passo a Passo de Implementação

### Passo 1: Crie o arquivo do conector em `crates/dendro-connectors/src/`
Crie, por exemplo, `crates/dendro-connectors/src/cursor.rs`:

```rust
pub struct CursorConnector {
    base_dir: std::path::PathBuf,
}

impl CursorConnector {
    pub fn new() -> Self {
        let home = std::env::var("HOME").unwrap_or_default();
        Self {
            base_dir: std::path::PathBuf::from(home).join(".cursor"),
        }
    }
}

#[async_trait::async_trait]
impl HarnessConnector for CursorConnector {
    fn name(&self) -> &'static str { "cursor" }
    // Implemente is_available, discover_sessions e extract_session...
}
```

### Passo 2: Exporte no `crates/dendro-connectors/src/lib.rs`
```rust
pub mod cursor;
pub use cursor::CursorConnector;
```

### Passo 3: Registre na lista de conectores do CLI (`crates/dendro-cli/src/main.rs`)
Adicione uma instância do seu conector no vetor padrão de ingestão executado por `dendro run` e `dendro watch`.

---

## 4. Testes de Conformidade do Conector
Crie um teste unitário com um arquivo de amostra mockado:
```rust
#[tokio::test]
async fn test_cursor_extraction() {
    let fixture_path = Path::new("tests/fixtures/cursor_session.json");
    let connector = CursorConnector::new();
    let session = connector.extract_session(fixture_path, None).await.unwrap();
    assert_eq!(session.harness, "cursor");
    assert!(session.input_tokens > 0);
}
```
