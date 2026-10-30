// Settings > Agent: the page title and intro, the agent add-on's sections
// (location 'ai': on a computer "Use the agent", "Where it runs", "What it
// may do", "On this computer" and Advanced; on a phone the hub-only
// version), then the recent Ask AI tasks from every device with their
// status and Retry, with the names of the computers that run them. MCP servers, quick setup and tokens are on Settings >
// Connectors.

import 'package:flutter/widgets.dart';

import '../../shell/addons.dart';
import '../../src/rust/api/ai.dart' show AiRunner, AiRunnerKind;
import '../../state/app_state.dart';
import '../../ui/ui.dart';
import '../settings/settings_kit.dart';
import 'ask_ai.dart';
import 'ai_store.dart';

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
    return ListenableBuilder(
      listenable: store,
      builder: (context, _) => SPage(
        title: 'Agent',
        intro:
            'The agent is Scratch’s built-in AI. It answers in the Agent chat and when you choose **Ask AI** from the / menu.',
        children: [
          for (final s in sections.where((s) => s.placement == 'top'))
            s.build(context),
          for (final s in sections.where((s) => s.placement == 'beforeTasks'))
            s.build(context),
          SSection(
            title: 'Computer names',
            intro:
                'The name each computer that runs tasks has in the vault. The Run on menu on the kanban board shows it.',
            children: [
              const SizedBox(height: 12),
              _RunnerNames(store: store),
            ],
          ),
          SSection(
            title: 'Recent tasks',
            intro: 'Ask AI requests from all your devices.',
            children: [
              const SizedBox(height: 12),
              _TaskList(store: store),
            ],
          ),
        ],
      ),
    );
  }
}

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
    return [
      for (final r in list.runners)
        if (r.kind == AiRunnerKind.pc) r,
    ];
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
          return SNotice(
            '${snap.error}',
            kind: SNoticeKind.bad,
            textKey: const ValueKey('runner-names-error'),
          );
        }
        if (!snap.hasData) {
          return Text('Loading computers…', style: sHint(context));
        }
        final pcs = snap.data!;
        if (pcs.isEmpty) {
          return const SEmpty(
            key: ValueKey('runner-names-empty'),
            icon: Sx.laptop,
            title: 'No computers yet',
            text:
                'Computers that run tasks appear here once they have signed in to the hub.',
          );
        }
        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            SList(children: [for (final r in pcs) _row(context, r, c)]),
            if (_error != null)
              SNotice(
                _error!,
                kind: SNoticeKind.bad,
                textKey: const ValueKey('runner-names-save-error'),
              ),
          ],
        );
      },
    );
  }

  Widget _row(BuildContext context, AiRunner r, AppColors c) {
    final editing = _editing == r.id;
    return KeyedSubtree(
      key: ValueKey('runner-name-row-${r.id}'),
      child: SItem(
        icon: Sx.laptop,
        name: r.name,
        nameWidget: editing
            ? SizedBox(
                width: 220,
                child: STextField(
                  controller: _name,
                  placeholder: 'Name',
                  fieldKey: ValueKey('runner-name-field-${r.id}'),
                  autofocus: true,
                  onSubmitted: (_) => _save(r),
                ),
              )
            : Text(
                r.name,
                key: ValueKey('runner-name-${r.id}'),
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: tsRaw(14, 19, 680, c.textPrimary),
              ),
        trailing: [
          if (editing)
            SButton(
              'Save',
              key: ValueKey('runner-name-save-${r.id}'),
              kind: SButtonKind.primary,
              onTap: _busy ? null : () => _save(r),
            )
          else
            SIconButton(
              Sx.pencil,
              key: ValueKey('runner-rename-${r.id}'),
              label: 'Rename ${r.name}',
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
    if (store.tasks.isEmpty) {
      return const SEmpty(
        key: ValueKey('ai-no-tasks'),
        icon: Sx.sparkles,
        title: 'No AI tasks yet',
        text: 'Select text, or type / on any page and choose Ask AI.',
      );
    }
    return SList(
      children: [
        for (final t in store.tasks.take(20))
          KeyedSubtree(
            key: ValueKey('ai-task-${t.id}'),
            child: SItem(
              icon: Sx.sparkles,
              name: t.prompt,
              sub: relativeTime(t.updatedMs.toInt()),
              error: t.error.isEmpty ? null : t.error,
              status: SStatus(
                aiStatusLabel(t.status),
                kind: switch (t.status) {
                  AiStatus.done => SStatusKind.ok,
                  AiStatus.failed => SStatusKind.bad,
                  AiStatus.running => SStatusKind.info,
                  _ => SStatusKind.off,
                },
              ),
              trailing: [
                if (t.status == AiStatus.failed)
                  SButton(
                    'Retry',
                    key: ValueKey('ai-retry-${t.id}'),
                    kind: SButtonKind.quiet,
                    onTap: () => store.retry(t.id),
                  ),
              ],
            ),
          ),
      ],
    );
  }
}
