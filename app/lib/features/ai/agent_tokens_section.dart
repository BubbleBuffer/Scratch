// Settings > AI > Agent tokens: the hub's agent tokens (name, access,
// created, last used, expiry), a form to create one, whose secret is shown
// once with a Copy button, and Revoke. Every signed-in device can manage
// them; the hub refuses agent tokens and OAuth sign-ins on these routes.
// Shown on every platform once this device is signed in to a hub.

import 'package:flutter/widgets.dart';

import '../../core/tool_sets.dart';
import '../../state/app_state.dart';
import '../../ui/toast.dart';
import '../../ui/ui.dart';
import '../settings/controls.dart';
import 'agent_token_client.dart';

/// Builds the client for this device, or null when it is not signed in to
/// an https:// hub. Tests pass their own.
typedef AgentTokenClientFactory = AgentTokenClient? Function(BuildContext);

AgentTokenClient? defaultAgentTokenClient(BuildContext context) {
  final app = context.app;
  final s = app.settings;
  final url = s.clientUrl?.trim() ?? '';
  final token = s.clientToken?.trim() ?? '';
  if (token.isEmpty || AgentTokenClient.hubUri(url) == null) return null;
  return AgentTokenClient(
    hubUrl: url,
    token: token,
    deviceId: app.vault.deviceId,
  );
}

String agentTokenExpiryLabel(int? days) => switch (days) {
  null => 'Never',
  365 => '1 year',
  _ => '$days days',
};

String agentTokenScopeLabel(String scope) => toolSetLabel(scope);

String _date(int ms) {
  final t = DateTime.fromMillisecondsSinceEpoch(ms);
  const months = [
    'Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun',
    'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec',
  ];
  return '${months[t.month - 1]} ${t.day}, ${t.year}';
}

/// "Read and write · Created Oct 3, 2026 · Last used 5m ago · Expires …".
String agentTokenSummary(AgentTokenInfo t) => [
  agentTokenScopeLabel(t.scope),
  'Created ${_date(t.created)}',
  t.lastUsed == null
      ? 'Never used'
      : 'Last used ${relativeTime(t.lastUsed!).toLowerCase()}',
  if (t.expired)
    'Expired ${_date(t.expires!)}'
  else if (t.expires == null)
    'Never expires'
  else
    'Expires ${_date(t.expires!)}',
].join(' · ');

class AgentTokensSection extends StatefulWidget {
  const AgentTokensSection({
    super.key,
    this.clientFor = defaultAgentTokenClient,
    this.top = 24,
  });

  final AgentTokenClientFactory clientFor;
  final double top;

  /// Replaces [clientFor] everywhere (tests).
  static AgentTokenClientFactory? debugClientFor;

  @override
  State<AgentTokensSection> createState() => _AgentTokensSectionState();
}

class _AgentTokensSectionState extends State<AgentTokensSection> {
  final _name = TextEditingController();
  AgentTokenClient? _client;
  AgentTokenList? _list;
  String? _error;
  bool _loading = false;
  bool _busy = false;
  bool _adding = false;
  String _scope = defaultTokenToolSet;
  int? _expiry = agentTokenDefaultExpiry;

  /// The secret of the token just created. Only in memory, and only until
  /// Done; it is never written anywhere.
  CreatedAgentToken? _created;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final next = (AgentTokensSection.debugClientFor ?? widget.clientFor)(
      context,
    );
    final changed =
        next?.hubUrl != _client?.hubUrl ||
        next?.token != _client?.token ||
        (next == null) != (_client == null);
    if (changed) {
      _client = next;
      _list = null;
      _error = null;
      _created = null;
      _load();
    } else if (_list == null && !_loading && _error == null) {
      _load();
    }
  }

  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  Future<void> _load() async {
    final client = _client;
    if (client == null) {
      // Called from didChangeDependencies too, so no setState here.
      _list = null;
      _error = null;
      return;
    }
    _loading = true;
    try {
      final list = await client.list();
      if (!mounted || !identical(client, _client)) return;
      setState(() {
        _list = list;
        _error = null;
        if (!list.expiryChoices.contains(_expiry)) {
          _expiry = list.defaultExpiry;
        }
      });
    } on AgentTokenException catch (e) {
      if (mounted && identical(client, _client)) {
        setState(() => _error = e.message);
      }
    } finally {
      _loading = false;
    }
  }

  Future<void> _create() async {
    final client = _client;
    final name = _name.text.trim();
    if (client == null || _busy) return;
    if (!agentTokenNamePattern.hasMatch(name)) {
      showErrorToast(
        context,
        'Use 1 to 40 lowercase letters, digits and dashes for the name.',
      );
      return;
    }
    setState(() => _busy = true);
    try {
      final made = await client.create(
        name: name,
        scope: _scope,
        expiresDays: _expiry,
      );
      if (!mounted) return;
      setState(() {
        _created = made;
        _adding = false;
        _name.clear();
      });
      await _load();
    } on AgentTokenException catch (e) {
      if (mounted) showErrorToast(context, e.message);
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  Future<void> _revoke(AgentTokenInfo t) async {
    final client = _client;
    if (client == null) return;
    final ok = await showConfirm(
      context,
      title: 'Revoke ${t.name}?',
      message:
          'Agents using this token lose access to the vault at once. This cannot be undone.',
      confirmLabel: 'Revoke',
      danger: true,
    );
    if (!ok || !mounted) return;
    try {
      await client.revoke(t.name);
      if (!mounted) return;
      if (_created?.info.name == t.name) setState(() => _created = null);
      showToast(context, 'Revoked ${t.name}');
      await _load();
    } on AgentTokenException catch (e) {
      if (mounted) showErrorToast(context, e.message);
    }
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final list = _list;
    final note = tsRaw(12, 16, 400, c.textSecondary);
    return Column(
      key: const ValueKey('agent-tokens'),
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        SettingsHeading('Agent tokens', top: widget.top),
        Text(
          'Tokens let Claude Code, Codex and other agents use this vault through the hub, over MCP or the notes command. Any device signed in to the hub can manage them.',
          style: tsRaw(13, 18, 400, c.textSecondary),
        ),
        const SizedBox(height: 12),
        if (_client == null)
          Text(
            'Sign in to an https:// hub in Sync to manage agent tokens.',
            key: const ValueKey('agent-tokens-signed-out'),
            style: note,
          )
        else ...[
          if (_created != null) _secret(context, _created!),
          if (_error != null)
            Row(
              children: [
                Expanded(
                  child: Text(
                    _error!,
                    key: const ValueKey('agent-tokens-error'),
                    style: tsRaw(12, 16, 400, c.inlineCodeText),
                  ),
                ),
                TextBtn(
                  'Retry',
                  key: const ValueKey('agent-tokens-retry'),
                  onTap: _load,
                ),
              ],
            ),
          if (list != null) ...[
            if (list.tokens.isEmpty)
              Text(
                'No agent tokens yet.',
                key: const ValueKey('agent-tokens-empty'),
                style: note,
              ),
            for (final t in list.tokens)
              SettingsRow(
                key: ValueKey('agent-token-${t.name}'),
                label: t.name,
                description: agentTokenSummary(t),
                trailing: TextBtn(
                  'Revoke',
                  key: ValueKey('agent-token-revoke-${t.name}'),
                  color: c.inlineCodeText,
                  onTap: () => _revoke(t),
                ),
              ),
            const SizedBox(height: 8),
            if (_adding)
              _form(context, list)
            else
              Align(
                alignment: Alignment.centerLeft,
                child: OutlineButton(
                  key: const ValueKey('agent-token-new'),
                  label: 'New token',
                  onTap: () => setState(() => _adding = true),
                ),
              ),
          ],
        ],
      ],
    );
  }

  Widget _secret(BuildContext context, CreatedAgentToken made) {
    final c = context.colors;
    return Container(
      key: const ValueKey('agent-token-created'),
      margin: const EdgeInsets.only(bottom: 12),
      padding: const EdgeInsets.all(12),
      decoration: BoxDecoration(
        color: c.bgSecondary,
        borderRadius: Radii.field,
        border: Border.all(color: c.borderSubtle),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(
            'Token ${made.info.name} created',
            style: tsRaw(14, 20, 500, c.textPrimary),
          ),
          const SizedBox(height: 4),
          Text(
            "Copy it now. The hub keeps only a hash, so it can't be shown again.",
            style: tsRaw(12, 16, 400, c.textSecondary),
          ),
          const SizedBox(height: 8),
          CopyableText(
            made.secret,
            textKey: const ValueKey('agent-token-secret'),
          ),
          const SizedBox(height: 8),
          Align(
            alignment: Alignment.centerLeft,
            child: TextBtn(
              'Done',
              key: const ValueKey('agent-token-secret-done'),
              onTap: () => setState(() => _created = null),
            ),
          ),
        ],
      ),
    );
  }

  Widget _form(BuildContext context, AgentTokenList list) {
    return Column(
      key: const ValueKey('agent-token-form'),
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        SettingsRow(
          label: 'Name',
          description: 'Lowercase letters, digits and dashes, such as laptop-claude.',
          stack: true,
          trailing: SettingsField(
            fieldKey: const ValueKey('agent-token-name'),
            controller: _name,
            mono: true,
            autofocus: true,
            placeholder: 'laptop-claude',
            onSubmitted: (_) => _create(),
          ),
        ),
        SettingsRow(
          label: 'Tool set',
          description:
              'Read only sees fewer tools and changes nothing. Everything also '
              'gets tools added later.',
          trailing: SelectButton<String>(
            key: const ValueKey('agent-token-scope'),
            title: 'Tool set',
            value: _scope,
            options: toolSets,
            labelOf: agentTokenScopeLabel,
            onChanged: (v) => setState(() => _scope = v),
          ),
        ),
        SettingsRow(
          label: 'Expires',
          description: 'After this the hub refuses the token. It stays listed until revoked.',
          trailing: SelectButton<int?>(
            key: const ValueKey('agent-token-expiry'),
            title: 'Expires',
            value: _expiry,
            options: list.expiryChoices,
            labelOf: (d) => d == null ? 'Never' : 'In ${agentTokenExpiryLabel(d)}',
            onChanged: (v) => setState(() => _expiry = v),
          ),
        ),
        const SizedBox(height: 8),
        Row(
          children: [
            SecondaryButton(
              key: const ValueKey('agent-token-create'),
              label: _busy ? 'Creating…' : 'Create token',
              height: 32,
              onTap: _busy ? null : _create,
            ),
            const SizedBox(width: 8),
            TextBtn(
              'Cancel',
              key: const ValueKey('agent-token-cancel'),
              onTap: () => setState(() {
                _adding = false;
                _name.clear();
              }),
            ),
          ],
        ),
      ],
    );
  }
}
