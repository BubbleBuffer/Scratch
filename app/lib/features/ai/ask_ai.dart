// "Ask AI": the box that asks for the prompt, opened from the `/` menu,
// the selection toolbar, the block menu and sheet, and the page menu. The
// run starts on the synced default runner ([AiStore.ask]: this computer or
// a hub job; phones use the Hub) and its answer goes into the page where
// the user asked.

import 'package:flutter/widgets.dart';

import '../../registry/block_registry.dart';
import '../../state/app_state.dart';
import '../../ui/ui.dart';
import '../settings/controls.dart';
import 'ai_settings.dart';
import 'ai_store.dart';

/// The `/` menu entry (not a block kind; the editor asks for the prompt).
const askAiEntry = BlockType(
  kind: 'askai',
  label: 'Ask AI',
  hint: 'Ask an AI agent to write here.',
  iconText: 'AI',
  aliases: 'ai assistant agent claude codex write',
  editable: false,
  insertOnly: true,
  colorable: false,
  group: 'AI',
);

void registerAiBlocks(BlockRegistry r) => r.register(askAiEntry);

/// Asks for a prompt and starts a run about [pageId]; its answer goes
/// after block [after] (an empty paragraph there is replaced) or at the
/// end. [selection] (selected text) goes into the prompt. Completes with
/// the run's id, or null if cancelled.
Future<String?> showAskAi(BuildContext context, {required String pageId, String? after, String? selection}) {
  final store = AiStore.of(context.app);
  return showModalPanel<String>(
    context,
    (ctx) => AskAiBox(
      store: store,
      selection: selection,
      onSubmit: (prompt) async {
        final id = await store.ask(askAiPrompt(prompt, selection), pageId, after: after);
        if (ctx.mounted) Navigator.of(ctx).pop(id);
      },
      onCancel: () => Navigator.of(ctx).pop(),
    ),
    width: 520,
    shrink: true,
  );
}

/// [prompt] about [selection] (selected text), quoted after it.
String askAiPrompt(String prompt, String? selection) {
  final sel = selection?.trim() ?? '';
  if (sel.isEmpty) return prompt;
  final quoted = sel.split('\n').map((l) => '> $l').join('\n');
  return '$prompt\n\nAbout this selected text:\n$quoted';
}

/// Where the answer comes from, under the prompt.
String askAiNote(AiStore s) {
  if (s.runsOn == AgentHost.hub) {
    if (s.hubLink == null) return 'Sign in to your hub in Settings > Sync first: the agent runs there.';
    return hubAgentPlatform
        ? 'The agent runs on your hub (phones always use it) and writes into the page.'
        : 'The agent runs on your hub and writes into the page.';
  }
  return s.canAnswer
      ? 'The agent answers on this device and writes into the page.'
      : 'A desktop with the agent on (Settings > AI) answers when it is open, and writes into the page.';
}

class AskAiBox extends StatefulWidget {
  const AskAiBox(
      {super.key, required this.store, required this.onSubmit, required this.onCancel, this.selection});
  final AiStore store;

  /// The selected text the prompt is about (shown above the field).
  final String? selection;
  final Future<void> Function(String prompt) onSubmit;
  final VoidCallback onCancel;

  @override
  State<AskAiBox> createState() => _AskAiBoxState();
}

class _AskAiBoxState extends State<AskAiBox> {
  final _prompt = TextEditingController();
  bool _sending = false;
  String? _error;

  @override
  void dispose() {
    _prompt.dispose();
    super.dispose();
  }

  Future<void> _submit() async {
    final p = _prompt.text.trim();
    if (p.isEmpty || _sending) return;
    setState(() {
      _sending = true;
      _error = null;
    });
    try {
      await widget.onSubmit(p);
    } catch (e) {
      if (mounted) {
        setState(() {
          _sending = false;
          _error = '$e';
        });
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final s = widget.store;
    final where = askAiNote(s);
    final sel = widget.selection?.trim() ?? '';
    return Padding(
      padding: const EdgeInsets.fromLTRB(20, 18, 20, 16),
      child: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        Row(children: [
          const AiMark(size: 18),
          const SizedBox(width: 8),
          Text('Ask AI', style: tsDisplay(const TypeSpec(19, 24, 640), c.textPrimary)),
        ]),
        const SizedBox(height: 12),
        if (sel.isNotEmpty) ...[
          Text(sel.length > 160 ? '“${sel.substring(0, 160)}…”' : '“$sel”',
              key: const ValueKey('ask-ai-selection'),
              maxLines: 3,
              overflow: TextOverflow.ellipsis,
              style: tsRaw(13, 18, 400, c.textSecondary)),
          const SizedBox(height: 8),
        ],
        SettingsField(
          fieldKey: const ValueKey('ask-ai-field'),
          controller: _prompt,
          placeholder: 'Write a summary, add a table of tasks, …',
          autofocus: true,
          onSubmitted: (_) => _submit(),
        ),
        const SizedBox(height: 8),
        Text(_error ?? where,
            key: const ValueKey('ask-ai-note'),
            style: tsRaw(12, 16, 400, _error != null ? c.inlineCodeText : c.textTertiary)),
        const SizedBox(height: 14),
        Row(mainAxisAlignment: MainAxisAlignment.end, children: [
          TextBtn('Cancel', onTap: widget.onCancel, color: c.textSecondary),
          const SizedBox(width: 8),
          PrimaryButton(key: const ValueKey('ask-ai-send'), label: _sending ? 'Asking…' : 'Ask', onTap: _submit),
        ]),
      ]),
    );
  }
}

/// The AI mark: a four-pointed sparkle.
class AiMark extends StatelessWidget {
  const AiMark({super.key, this.size = 20, this.color});
  final double size;
  final Color? color;

  @override
  Widget build(BuildContext context) => SizedBox(
        width: size,
        height: size,
        child: CustomPaint(painter: _SparklePainter(color ?? context.colors.iconPrimary)),
      );
}

class _SparklePainter extends CustomPainter {
  _SparklePainter(this.color);
  final Color color;

  @override
  void paint(Canvas canvas, Size size) {
    final k = size.width / 20;
    canvas.scale(k);
    final fill = Paint()..color = color;
    Path star(double cx, double cy, double r, double w) => Path()
      ..moveTo(cx, cy - r)
      ..quadraticBezierTo(cx + w, cy - w, cx + r, cy)
      ..quadraticBezierTo(cx + w, cy + w, cx, cy + r)
      ..quadraticBezierTo(cx - w, cy + w, cx - r, cy)
      ..quadraticBezierTo(cx - w, cy - w, cx, cy - r)
      ..close();
    canvas.drawPath(star(8.5, 11, 7, 1.4), fill);
    canvas.drawPath(star(15.5, 4.5, 3, 0.6), fill);
  }

  @override
  bool shouldRepaint(_SparklePainter old) => old.color != color;
}

/// A task's status in words.
String aiStatusLabel(String status) => switch (status) {
      AiStatus.queued => 'Queued',
      AiStatus.running => 'Running',
      AiStatus.done => 'Done',
      AiStatus.failed => 'Failed',
      AiStatus.cancelled => 'Cancelled',
      AiStatus.interrupted => 'Interrupted',
      _ => status,
    };
