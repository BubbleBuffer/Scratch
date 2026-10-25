// Settings > AI: the agent first (an add-on section placed 'top', with "Run
// AI on" and what that runner uses), then where Ask AI runs (the agent on
// the hub or on this computer, per "Run AI on"), the vault's MCP endpoint
// for AI apps, the hub's agent tokens, the names of the computers that run
// tasks, and the recent tasks with their status. Phones only list tasks:
// computers answer them.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../../src/rust/api/ai.dart' show AiRunner, AiRunnerKind;
import '../../state/app_state.dart';
import '../../ui/ui.dart';
import '../settings/controls.dart';
import '../../shell/addons.dart';
import 'agent_tokens_section.dart';
import 'ai_settings.dart';
import 'ai_store.dart';
import 'ask_ai.dart';
import 'connect_agent_section.dart';

class AiSettingsBody extends StatelessWidget {
  const AiSettingsBody({super.key});

  @override
  Widget build(BuildContext context) {
    final store = AiStore.of(context.app);
    final sections = appAddonContext.settingsSections.all
        .where(
          (s) => s.location == 'ai' && (s.isVisible?.call(context) ?? true),
        )
        .toList();
    final beforeTasks = sections.where((s) => s.placement == 'beforeTasks');
    return ListenableBuilder(
      listenable: store,
      builder: (context, _) {
        final c = context.colors;
        final m = context.mobile;
        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            const SettingsHeading('AI', primary: true),
            Text(
              'Chat with the agent, or ask AI from the / menu on any device.',
              style: m
                  ? tsRaw(15, 21, 400, c.textSecondary)
                  : tsRaw(14, 20, 400, c.textSecondary),
            ),
            const SizedBox(height: 16),
            if (AiStore.canRun) ...[
              for (final section in sections.where(
                (s) => s.placement == 'top',
              ))
                section.build(context),
              const SettingsHeading('Ask AI', top: 24),
              // Ask AI always uses the agent, where "Run AI on" says: a hub
              // job, or this computer's agent.
              if (store.runsOn == AgentHost.hub)
                Text(
                  'Ask AI runs on the hub.',
                  key: const ValueKey('ai-runs-on-hub'),
                  style: m
                      ? tsRaw(15, 21, 400, c.textSecondary)
                      : tsRaw(14, 20, 400, c.textSecondary),
                )
              else
                Text(
                  'Ask AI runs on this computer.',
                  key: const ValueKey('ai-runs-on-this-computer'),
                  style: m
                      ? tsRaw(15, 21, 400, c.textSecondary)
                      : tsRaw(14, 20, 400, c.textSecondary),
                ),
              const SettingsHeading('MCP endpoint', top: 24),
              _McpSection(store: store),
            ],
            for (final section in beforeTasks) section.build(context),
            // Its `notes setup` commands run in a desktop terminal.
            if (!m)
              ConnectAgentSection(
                top: AiStore.canRun || beforeTasks.isNotEmpty ? 24 : 0,
              ),
            AgentTokensSection(
              top: !m || AiStore.canRun || beforeTasks.isNotEmpty ? 24 : 0,
            ),
            const SettingsHeading('Runner names', top: 24),
            _RunnerNames(store: store),
            const SettingsHeading('Recent tasks', top: 24),
            _TaskList(store: store),
          ],
        );
      },
    );
  }
}

class _McpSection extends StatefulWidget {
  const _McpSection({required this.store});
  final AiStore store;

  @override
  State<_McpSection> createState() => _McpSectionState();
}

class _McpSectionState extends State<_McpSection> {
  late final _addr = TextEditingController(text: widget.store.settings.mcpAddress);
  String? _copied;

  @override
  void dispose() {
    _addr.dispose();
    super.dispose();
  }

  Future<void> _copy(String what, Future<String> Function() text) async {
    await Clipboard.setData(ClipboardData(text: await text()));
    if (!mounted) return;
    setState(() => _copied = what);
    await Future<void>.delayed(const Duration(seconds: 2));
    if (mounted && _copied == what) setState(() => _copied = null);
  }

  @override
  Widget build(BuildContext context) {
    final store = widget.store;
    final s = store.settings;
    final c = context.colors;
    final url = store.mcpRunning ? store.mcpUrl! : 'http://${s.mcpAddress}/mcp';
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      SettingsRow(
        label: 'Serve the vault to AI apps',
        description: 'Lets Claude, Codex and other MCP clients on this computer read and edit the vault while the app is open. They need the token.',
        trailing: Toggle(key: const ValueKey('ai-mcp-toggle'), value: s.mcpEnabled, onChanged: store.setMcpEnabled),
      ),
      if (s.mcpEnabled) ...[
        SettingsRow(
          label: 'Address',
          description: 'Keep 127.0.0.1 (this computer only); reach it from elsewhere through a tunnel with HTTPS.',
          stack: true,
          trailing: SettingsField(
            fieldKey: const ValueKey('ai-mcp-address'),
            controller: _addr,
            mono: true,
            onSubmitted: (v) => store.setMcpAddress(v.trim()),
          ),
        ),
        if (store.mcpError != null)
          Text(store.mcpError!, key: const ValueKey('ai-mcp-error'), style: tsRaw(12, 16, 400, c.inlineCodeText)),
        if (store.mcpRunning) ...[
          const SizedBox(height: 4),
          CopyableText(url, textKey: const ValueKey('ai-mcp-url')),
          const SizedBox(height: 10),
          Wrap(spacing: 8, runSpacing: 8, children: [
            OutlineButton(
              key: const ValueKey('ai-copy-token'),
              label: _copied == 'token' ? 'Copied' : 'Copy token',
              onTap: () => _copy('token', store.token),
            ),
            OutlineButton(
              key: const ValueKey('ai-copy-claude'),
              label: _copied == 'claude' ? 'Copied' : 'Copy Claude Code command',
              onTap: () => _copy('claude', () async => claudeAddCommand(url, await store.token())),
            ),
            OutlineButton(
              key: const ValueKey('ai-new-token'),
              label: 'New token',
              onTap: () async {
                if (await showConfirm(context,
                    title: 'Make a new token?',
                    message: 'AI apps set up with the current token stop working until you give them the new one.',
                    confirmLabel: 'New token')) {
                  await store.rotateToken();
                }
              },
            ),
          ]),
        ],
      ],
    ]);
  }
}

/// The command that adds the endpoint to Claude Code.
String claudeAddCommand(String url, String token) =>
    'claude mcp add --transport http notes $url --header "Authorization: Bearer $token"';

/// The computers that can run tasks, each with the name it has in the vault.
/// Renaming one changes the name the kanban "Run on" picker shows.
class _RunnerNames extends StatefulWidget {
  const _RunnerNames({required this.store});
  final AiStore store;

  @override
  State<_RunnerNames> createState() => _RunnerNamesState();
}

class _RunnerNamesState extends State<_RunnerNames> {
  late Future<List<AiRunner>> _pcs = _load();
  final _name = TextEditingController();
  String? _editing;
  String? _error;
  bool _busy = false;

  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  /// The PCs of the vault's runner list (the hub has no name to change).
  Future<List<AiRunner>> _load() async {
    final list = await widget.store.runnerBridge.listRunners(
      hubUsable: widget.store.hubUsable,
    );
    return [for (final r in list.runners) if (r.kind == AiRunnerKind.pc) r];
  }

  void _startEdit(AiRunner r) => setState(() {
    _editing = r.id;
    _name.text = r.name;
    _error = null;
  });

  Future<void> _save(AiRunner r) async {
    final name = _name.text.trim();
    if (name.isEmpty) {
      setState(() => _error = 'Enter a name.');
      return;
    }
    setState(() => _busy = true);
    try {
      await widget.store.runnerBridge.rename(deviceId: r.id, name: name);
      await widget.store.refresh();
      if (!mounted) return;
      setState(() {
        _editing = null;
        _busy = false;
        _pcs = _load();
      });
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _error = '$e';
        _busy = false;
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return FutureBuilder<List<AiRunner>>(
      future: _pcs,
      builder: (context, snap) {
        if (snap.hasError) {
          return Text(
            '${snap.error}',
            key: const ValueKey('runner-names-error'),
            style: tsRaw(14, 20, 400, c.textSecondary),
          );
        }
        if (!snap.hasData) {
          return Text(
            'Loading computers…',
            style: tsRaw(14, 20, 400, c.textTertiary),
          );
        }
        final pcs = snap.data!;
        if (pcs.isEmpty) {
          return Text(
            'No computers yet.',
            key: const ValueKey('runner-names-empty'),
            style: tsRaw(14, 20, 400, c.textTertiary),
          );
        }
        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            for (final r in pcs) _row(context, r),
            if (_error != null)
              Padding(
                padding: const EdgeInsets.only(top: 8),
                child: Text(
                  _error!,
                  key: const ValueKey('runner-names-save-error'),
                  style: tsRaw(14, 20, 400, c.textSecondary),
                ),
              ),
          ],
        );
      },
    );
  }

  Widget _row(BuildContext context, AiRunner r) {
    final c = context.colors;
    final editing = _editing == r.id;
    return Padding(
      key: ValueKey('runner-name-row-${r.id}'),
      padding: const EdgeInsets.symmetric(vertical: 6),
      child: Row(
        children: [
          Expanded(
            child: editing
                ? SettingsField(
                    controller: _name,
                    placeholder: 'Name',
                    fieldKey: ValueKey('runner-name-field-${r.id}'),
                    autofocus: true,
                    onSubmitted: (_) => _save(r),
                  )
                : Text(
                    r.name,
                    key: ValueKey('runner-name-${r.id}'),
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: tsRaw(15, 21, 400, c.textPrimary),
                  ),
          ),
          const SizedBox(width: 8),
          if (editing)
            PrimaryButton(
              key: ValueKey('runner-name-save-${r.id}'),
              label: 'Save',
              height: 38,
              onTap: _busy ? () {} : () => _save(r),
            )
          else
            IconBtn(
              Ico.compose,
              key: ValueKey('runner-rename-${r.id}'),
              tooltip: 'Rename ${r.name}',
              onTap: () => _startEdit(r),
            ),
        ],
      ),
    );
  }
}

class _TaskList extends StatelessWidget {
  const _TaskList({required this.store});
  final AiStore store;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final m = context.mobile;
    if (store.tasks.isEmpty) {
      return Padding(
        padding: const EdgeInsets.symmetric(vertical: 12),
        child: Text('No AI tasks yet.', key: const ValueKey('ai-no-tasks'), style: tsRaw(14, 20, 400, c.textTertiary)),
      );
    }
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      for (final t in store.tasks.take(20))
        Padding(
          key: ValueKey('ai-task-${t.id}'),
          padding: const EdgeInsets.symmetric(vertical: 6),
          child: Row(crossAxisAlignment: CrossAxisAlignment.start, children: [
            _StatusChip(t.status),
            const SizedBox(width: 10),
            Expanded(
              child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
                Text(t.prompt,
                    maxLines: 2,
                    overflow: TextOverflow.ellipsis,
                    style: m ? tsRaw(16, 22, 400, c.textPrimary) : tsRaw(14, 20, 400, c.textPrimary)),
                if (t.error.isNotEmpty)
                  Text(t.error,
                      maxLines: 3, overflow: TextOverflow.ellipsis, style: tsRaw(12, 16, 400, c.inlineCodeText)),
                Text(relativeTime(t.updatedMs.toInt()), style: tsRaw(12, 16, 400, c.textTertiary)),
              ]),
            ),
            if (t.status == AiStatus.failed) ...[
              const SizedBox(width: 8),
              OutlineButton(key: ValueKey('ai-retry-${t.id}'), label: 'Retry', onTap: () => store.retry(t.id)),
            ],
          ]),
        ),
    ]);
  }
}

class _StatusChip extends StatelessWidget {
  const _StatusChip(this.status);
  final String status;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final bg = switch (status) {
      AiStatus.done => c.okBg,
      AiStatus.failed => c.dangerBg,
      AiStatus.running => c.infoBg,
      _ => c.bgSecondary,
    };
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 1),
      decoration: BoxDecoration(color: bg, borderRadius: Radii.pill),
      child: Text(aiStatusLabel(status), style: tsRaw(12, 18, 600, c.textPrimary)),
    );
  }
}
