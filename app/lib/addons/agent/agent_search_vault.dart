import '../../core/vault.dart';
import '../../src/rust/api/types.dart' show ExternalSearchRow;
import 'agent_search_source.dart';

/// The vault's search index as an [AgentSearchSink]. [AgentStore.searchSinkFor]
/// uses it by default.
class VaultAgentSearchSink implements AgentSearchSink {
  VaultAgentSearchSink(this.vault);

  final Vault vault;

  @override
  Future<void> upsert(AgentSearchRow row) => vault.run(
    (v) => v.upsertSearchRow(
      source: agentSearchSource,
      row: ExternalSearchRow(
        id: row.id,
        title: row.title,
        body: row.body,
        pageType: agentSearchPageType,
        target: row.target,
        updatedMs: row.updatedMs,
      ),
    ),
  );

  @override
  Future<void> delete(String id) =>
      vault.run((v) => v.deleteSearchRow(source: agentSearchSource, id: id));

  @override
  Future<void> clear() =>
      vault.run((v) => v.clearSearchSource(source: agentSearchSource));
}
