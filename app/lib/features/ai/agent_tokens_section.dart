// Settings > Connectors > Apps that use your vault > Access tokens: the
// hub's agent tokens (name, access, created, last used, expiry), a form to
// create one, whose secret is shown once with a Copy button, and Revoke. Every signed-in device can manage
// them; the hub refuses agent tokens and OAuth sign-ins on these routes.
// Shown on every platform once this device is signed in to a hub.

import 'package:flutter/widgets.dart';

import '../../core/tool_sets.dart';
import '../../state/app_state.dart';
import '../../ui/toast.dart';
import '../../ui/ui.dart';
import '../settings/settings_kit.dart';
import '../settings/settings_screen.dart';
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
    this.first = false,
    this.onCount,
  });

  final AgentTokenClientFactory clientFor;

  /// Drops the section's top margin.
  final bool first;

  /// Told the number of tokens after each load, or null when signed out or
  /// the list failed (the Connectors panel's status).
  final ValueChanged<int?>? onCount;

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
      _count(null);
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
      _count(list.tokens.length);
    } on AgentTokenException catch (e) {
      if (mounted && identical(client, _client)) {
        setState(() => _error = e.message);
        _count(null);
      }
    } finally {
      _loading = false;
    }
  }

  /// Reports the count after this frame: [_load] runs during build too.
  void _count(int? n) {
    final f = widget.onCount;
    if (f == null) return;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) f(n);
    });
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
    final list = _list;
    final Widget body;
    if (_client == null) {
      body = SEmpty(
        key: const ValueKey('agent-tokens-signed-out'),
        icon: Sx.key,
        title: 'Sign in to a hub first',
        text: 'Tokens are issued by your hub. Sign in under Sync and devices.',
        action: SButton(
          'Open Sync and devices',
          key: const ValueKey('agent-tokens-open-sync'),
          onTap: () => openSettingsSection(context, SettingsSection.sync),
        ),
      );
    } else {
      Widget newButton({bool primary = false}) => SButton(
        'New token',
        key: const ValueKey('agent-token-new'),
        icon: Sx.plus,
        kind: primary ? SButtonKind.primary : SButtonKind.normal,
        onTap: () => setState(() => _adding = true),
      );
      body = Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          if (_created != null) _secret(context, _created!),
          if (_error != null)
            SNotice(
              _error!,
              kind: SNoticeKind.bad,
              textKey: const ValueKey('agent-tokens-error'),
              actions: [
                SButton(
                  'Retry',
                  key: const ValueKey('agent-tokens-retry'),
                  kind: SButtonKind.quiet,
                  onTap: _load,
                ),
              ],
            ),
          if (list != null) ...[
            if (list.tokens.isEmpty && !_adding)
              SEmpty(
                key: const ValueKey('agent-tokens-empty'),
                icon: Sx.key,
                title: 'No tokens yet',
                text:
                    'Make one token per AI app, so you can cut off one app without touching the others.',
                action: newButton(primary: true),
              )
            else if (list.tokens.isNotEmpty)
              SList(
                foot: _adding ? null : SListFoot(children: [newButton()]),
                children: [
                  for (final t in list.tokens)
                    KeyedSubtree(
                      key: ValueKey('agent-token-${t.name}'),
                      child: SItem(
                        icon: Sx.key,
                        name: t.name,
                        tags: [agentTokenScopeLabel(t.scope)],
                        sub: agentTokenSummary(t),
                        trailing: [
                          SButton(
                            'Revoke',
                            key: ValueKey('agent-token-revoke-${t.name}'),
                            kind: SButtonKind.danger,
                            onTap: () => _revoke(t),
                          ),
                        ],
                      ),
                    ),
                ],
              ),
            if (_adding) _form(context, list),
          ],
        ],
      );
    }
    return KeyedSubtree(
      key: const ValueKey('agent-tokens'),
      child: SSection(
        first: widget.first,
        title: 'Access tokens',
        intro:
            'Each token lets one app reach your vault through your hub, over MCP or the notes command. Revoke a token to cut that app off. Any device signed in to the hub can manage them.',
        children: [const SizedBox(height: 12), body],
      ),
    );
  }

  Widget _secret(BuildContext context, CreatedAgentToken made) {
    return Padding(
      key: const ValueKey('agent-token-created'),
      padding: const EdgeInsets.only(bottom: 12),
      child: SNotice(
        'Token **${made.info.name}** created. Copy it now. The hub keeps only a hash, so it can’t be shown again.',
        actions: [
          SButton(
            'Done',
            key: const ValueKey('agent-token-secret-done'),
            kind: SButtonKind.quiet,
            onTap: () => setState(() => _created = null),
          ),
        ],
        below: SCopyLine(
          made.secret,
          textKey: const ValueKey('agent-token-secret'),
        ),
      ),
    );
  }

  Widget _form(BuildContext context, AgentTokenList list) {
    return SEditor(
      key: const ValueKey('agent-token-form'),
      title: 'New token',
      actions: [
        SButton(
          'Cancel',
          key: const ValueKey('agent-token-cancel'),
          kind: SButtonKind.quiet,
          onTap: () => setState(() {
            _adding = false;
            _name.clear();
          }),
        ),
        SButton(
          _busy ? 'Creating…' : 'Create token',
          key: const ValueKey('agent-token-create'),
          kind: SButtonKind.primary,
          onTap: _busy ? null : _create,
        ),
      ],
      children: [
        SField(
          label: 'Name',
          hint: 'Lowercase letters, digits and dashes.',
          child: STextField(
            fieldKey: const ValueKey('agent-token-name'),
            controller: _name,
            mono: true,
            autofocus: true,
            placeholder: 'laptop-claude',
            onSubmitted: (_) => _create(),
          ),
        ),
        SField(
          label: 'Expires',
          hint: 'After this the hub refuses the token. It stays listed until revoked.',
          child: Align(
            alignment: Alignment.centerLeft,
            child: SSelect<int?>(
              key: const ValueKey('agent-token-expiry'),
              title: 'Expires',
              value: _expiry,
              options: list.expiryChoices,
              labelOf: (d) =>
                  d == null ? 'Never' : 'In ${agentTokenExpiryLabel(d)}',
              onChanged: (v) => setState(() => _expiry = v),
            ),
          ),
        ),
        SField(
          label: 'What the app may do',
          hint:
              'Read changes nothing. Everything also gets tools added later. The secret is shown once, right after you create the token.',
          child: Align(
            alignment: Alignment.centerLeft,
            child: SSegmented<String>(
              key: const ValueKey('agent-token-scope'),
              value: _scope,
              options: toolSets,
              keyOf: (s) => ValueKey('agent-token-scope-$s'),
              labelOf: agentTokenScopeLabel,
              onChanged: (v) => setState(() => _scope = v),
            ),
          ),
        ),
      ],
    );
  }
}
