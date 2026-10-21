import 'dart:convert';

import 'package:flutter/widgets.dart';
import 'package:flutter/services.dart';

import '../../ui/ui.dart';
import 'agent_store.dart';
import 'agent_transcript.dart';
import 'agent_markdown.dart';

class CodeCellRow extends StatefulWidget {
  const CodeCellRow({super.key, required this.cell});
  final CodeCell cell;

  @override
  State<CodeCellRow> createState() => _CodeCellRowState();
}

class _CodeCellRowState extends State<CodeCellRow> {
  bool _open = false;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final cell = widget.cell;
    final (label, bg) = switch (cell.status) {
      CellStatus.running => ('Running', c.infoBg),
      CellStatus.done => ('Done', c.okBg),
      CellStatus.failed => ('Failed', c.dangerBg),
    };
    return Container(
      decoration: BoxDecoration(
        border: Border.all(color: c.border),
        borderRadius: Radii.field,
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Hover(
            key: ValueKey('agent-${cell.id}-toggle'),
            onTap: () => setState(() => _open = !_open),
            borderRadius: Radii.field,
            padding: const EdgeInsets.symmetric(horizontal: 8),
            child: SizedBox(
              height: 34,
              child: Row(
                children: [
                  Icon20(
                    _open ? Ico.chevronDown : Ico.chevronRight,
                    size: 14,
                    color: c.iconTertiary,
                  ),
                  const SizedBox(width: 6),
                  Icon20(Ico.code, size: 16, color: c.iconSecondary),
                  const SizedBox(width: 8),
                  Expanded(
                    child: Text(
                      cell.firstLine,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: ts(context.type.code, c.textSecondary, mono: true),
                    ),
                  ),
                  const SizedBox(width: 8),
                  if (cell.status == CellStatus.running) ...[
                    Spinner(size: 12, color: c.iconTertiary),
                    const SizedBox(width: 6),
                  ],
                  Container(
                    padding: const EdgeInsets.symmetric(
                      horizontal: 6,
                      vertical: 1,
                    ),
                    decoration: BoxDecoration(
                      color: bg,
                      borderRadius: Radii.pill,
                    ),
                    child: Text(
                      label,
                      style: tsRaw(12, 18, 600, c.textPrimary),
                    ),
                  ),
                ],
              ),
            ),
          ),
          if (_open)
            Padding(
              padding: const EdgeInsets.fromLTRB(8, 0, 8, 8),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  CodeBox(
                    cell.code,
                    textKey: ValueKey('agent-${cell.id}-code'),
                  ),
                  if (cell.result != null) ...[
                    const SizedBox(height: 8),
                    Text(
                      'Result',
                      style: ts(context.type.uiSmall, c.textTertiary),
                    ),
                    const SizedBox(height: 4),
                    CodeBox(
                      cell.result!,
                      textKey: ValueKey('agent-${cell.id}-result'),
                      color: cell.status == CellStatus.failed
                          ? c.inlineCodeText
                          : null,
                    ),
                  ],
                ],
              ),
            ),
        ],
      ),
    );
  }
}

String? _projectOpenText(String arguments) {
  try {
    final args = jsonDecode(arguments) as Map<String, dynamic>;
    final path = args['path'] as String;
    final name = (args['name'] as String?)?.trim();
    final label = name == null || name.isEmpty
        ? path.split(RegExp(r'[/\\]')).where((p) => p.isNotEmpty).lastOrNull ??
              path
        : name;
    return 'Open folder $path as project $label?';
  } catch (_) {
    return null;
  }
}

/// A tool call waiting for a decision: the tool, its arguments, and
/// Approve, Approve for session and Deny (with an optional reason).
class ApprovalCard extends StatefulWidget {
  const ApprovalCard({
    super.key,
    required this.approval,
    required this.onDecide,
  });
  final AgentApproval approval;

  /// `approve`, `approve_session` or `deny` (with the reason, if any).
  final void Function(String decision, String? reason) onDecide;

  @override
  State<ApprovalCard> createState() => _ApprovalCardState();
}

class _ApprovalCardState extends State<ApprovalCard> {
  bool _denying = false;
  final _reason = TextEditingController();
  final _reasonFocus = FocusNode();

  @override
  void dispose() {
    _reason.dispose();
    _reasonFocus.dispose();
    super.dispose();
  }

  void _deny() {
    final r = _reason.text.trim();
    widget.onDecide('deny', r.isEmpty ? null : r);
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final a = widget.approval;
    final id = a.id;
    return Container(
      padding: const EdgeInsets.fromLTRB(16, 14, 16, 14),
      decoration: BoxDecoration(
        color: c.bgPrimary,
        borderRadius: Radii.panel,
        border: Border.all(color: c.thread),
        boxShadow: c.seam,
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Row(
            children: [
              Container(
                padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 1),
                decoration: BoxDecoration(
                  color: c.attnBg,
                  borderRadius: Radii.pill,
                ),
                child: Text(
                  'Needs approval',
                  style: tsRaw(12, 18, 600, c.textPrimary),
                ),
              ),
              const SizedBox(width: 10),
              Expanded(
                child: Text(
                  a.tool,
                  key: ValueKey('agent-approval-tool-$id'),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: ts(
                    context.type.ui,
                    c.textPrimary,
                    weight: 700,
                    mono: true,
                  ),
                ),
              ),
            ],
          ),
          const SizedBox(height: 10),
          if (a.tool == 'project_open' &&
              _projectOpenText(a.argumentsJson) != null)
            Text(
              _projectOpenText(a.argumentsJson)!,
              key: ValueKey('agent-approval-args-$id'),
              style: ts(context.type.body, c.textPrimary),
            )
          else
            CodeBox(
              prettyJson(a.argumentsJson),
              textKey: ValueKey('agent-approval-args-$id'),
            ),
          const SizedBox(height: 12),
          if (!_denying)
            Wrap(
              spacing: 8,
              runSpacing: 8,
              children: [
                PrimaryButton(
                  key: ValueKey('agent-approve-$id'),
                  label: 'Approve',
                  height: 32,
                  onTap: () => widget.onDecide('approve', null),
                ),
                IntrinsicWidth(
                  child: SecondaryButton(
                    key: ValueKey('agent-approve-session-$id'),
                    label: 'Approve for session',
                    height: 32,
                    onTap: () => widget.onDecide('approve_session', null),
                  ),
                ),
                IntrinsicWidth(
                  child: SecondaryButton(
                    key: ValueKey('agent-deny-$id'),
                    label: 'Deny',
                    height: 32,
                    onTap: () {
                      setState(() => _denying = true);
                      _reasonFocus.requestFocus();
                    },
                  ),
                ),
              ],
            )
          else
            Row(
              children: [
                Expanded(
                  child: FieldFrame(
                    height: 32,
                    child: PlainField(
                      key: ValueKey('agent-deny-reason-$id'),
                      controller: _reason,
                      focusNode: _reasonFocus,
                      style: ts(context.type.ui, c.textPrimary, weight: 400),
                      placeholder: 'Reason (optional)',
                      textInputAction: TextInputAction.done,
                      onSubmitted: (_) => _deny(),
                    ),
                  ),
                ),
                const SizedBox(width: 8),
                SecondaryButton(
                  key: ValueKey('agent-deny-confirm-$id'),
                  label: 'Deny',
                  height: 32,
                  onTap: _deny,
                ),
                const SizedBox(width: 4),
                TextBtn(
                  'Cancel',
                  height: 32,
                  onTap: () => setState(() => _denying = false),
                ),
              ],
            ),
        ],
      ),
    );
  }
}
