// Settings > AI: who answers AI tasks on this device (Claude Code, Codex,
// DeepSeek), the vault's MCP endpoint for AI apps, and the recent tasks
// with their status. Phones only list tasks: desktops answer them.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../../state/app_state.dart';
import '../../ui/ui.dart';
import '../settings/controls.dart';
import '../../shell/addons.dart';
import 'ai_settings.dart';
import 'ai_store.dart';
import 'ask_ai.dart';

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
              'Ask AI from the / menu on any device. A desktop with a runner set up answers and writes into the page.',
              style: m
                  ? tsRaw(15, 21, 400, c.textSecondary)
                  : tsRaw(14, 20, 400, c.textSecondary),
            ),
            const SizedBox(height: 16),
            if (AiStore.canRun) ...[
              _RunnerSection(store: store),
              for (final section in sections.where(
                (s) => s.placement == 'afterRunner',
              ))
                section.build(context),
              const SettingsHeading('MCP endpoint', top: 24),
              _McpSection(store: store),
            ],
            for (final section in beforeTasks) section.build(context),
            SettingsHeading(
              'Recent tasks',
              top: AiStore.canRun || beforeTasks.isNotEmpty ? 24 : 0,
            ),
            _TaskList(store: store),
          ],
        );
      },
    );
  }
}

class _RunnerSection extends StatefulWidget {
  const _RunnerSection({required this.store});
  final AiStore store;

  @override
  State<_RunnerSection> createState() => _RunnerSectionState();
}

class _RunnerSectionState extends State<_RunnerSection> {
  late final _claude = TextEditingController(text: widget.store.settings.claudePath);
  late final _codex = TextEditingController(text: widget.store.settings.codexPath);
  late final _model = TextEditingController(text: widget.store.settings.deepSeekModel);
  late final _url = TextEditingController(text: widget.store.settings.deepSeekUrl);
  final _key = TextEditingController();

  @override
  void dispose() {
    for (final c in [_claude, _codex, _model, _url, _key]) {
      c.dispose();
    }
    super.dispose();
  }

  void _set(void Function(AiSettings s) f) {
    widget.store.settings.update(f);
    widget.store.pump();
  }

  @override
  Widget build(BuildContext context) {
    final s = widget.store.settings;
    final c = context.colors;
    Widget path(String label, TextEditingController ctl, String name, void Function(AiSettings, String) save) =>
        SettingsRow(
          label: label,
          description: 'Full path of the program. Leave empty to use $name from PATH.',
          stack: true,
          trailing: SettingsField(
            fieldKey: ValueKey('ai-path-$name'),
            controller: ctl,
            mono: true,
            placeholder: name,
            onChanged: (v) => _set((s) => save(s, v.trim())),
          ),
        );
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      SettingsRow(
        label: 'Runner',
        description: 'Who answers AI tasks on this device. The agent, Claude Code and Codex run here and can read and edit the vault; DeepSeek only sees the page.',
        trailing: SelectButton<AiRunner>(
          key: const ValueKey('ai-runner'),
          title: 'Runner',
          value: s.runner,
          options: [
            for (final r in AiRunner.values)
              if (r != AiRunner.agent || s.agentEnabled || s.runner == AiRunner.agent) r,
          ],
          labelOf: (r) => r.label,
          onChanged: (r) => _set((s) => s.runner = r),
        ),
      ),
      if (s.runner == AiRunner.claude) path('Claude Code program', _claude, 'claude', (s, v) => s.claudePath = v),
      if (s.runner == AiRunner.codex) path('Codex program', _codex, 'codex', (s, v) => s.codexPath = v),
      if (s.runner == AiRunner.deepseek) ...[
        if (s.deepSeekKey.isEmpty)
          SettingsRow(
            label: 'DeepSeek API key',
            description: 'Kept on this device only.',
            stack: true,
            trailing: Row(children: [
              Expanded(child: SettingsField(fieldKey: const ValueKey('ai-deepseek-key'), controller: _key, mono: true)),
              const SizedBox(width: 8),
              OutlineButton(
                key: const ValueKey('ai-deepseek-save'),
                label: 'Save',
                onTap: () {
                  final k = _key.text.trim();
                  if (k.isEmpty) return;
                  _key.clear();
                  _set((s) => s.deepSeekKey = k);
                },
              ),
            ]),
          )
        else
          SettingsRow(
            label: 'DeepSeek API key',
            description: 'Saved on this device (ends in ${_tail(s.deepSeekKey)}).',
            trailing: OutlineButton(
              key: const ValueKey('ai-deepseek-remove'),
              label: 'Remove',
              danger: true,
              onTap: () => _set((s) => s.deepSeekKey = ''),
            ),
          ),
        SettingsRow(
          label: 'Model',
          stack: true,
          trailing: SettingsField(
              controller: _model, mono: true, onChanged: (v) => _set((s) => s.deepSeekModel = v.trim())),
        ),
        SettingsRow(
          label: 'API address',
          stack: true,
          trailing: SettingsField(
              controller: _url, mono: true, onChanged: (v) => _set((s) => s.deepSeekUrl = v.trim())),
        ),
      ],
      if (s.runner != AiRunner.none && !s.ready(s.runner))
        Text(s.runner == AiRunner.agent ? 'Turn on the agent below to run tasks.' : 'Add the API key to run tasks.',
            style: tsRaw(12, 16, 400, c.inlineCodeText)),
    ]);
  }

  static String _tail(String k) => k.length <= 4 ? k : k.substring(k.length - 4);
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
