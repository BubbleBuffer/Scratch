import '../../src/rust/api/agent.dart';
import 'outline_names.dart';

/// Display helpers keep the complete decisions in the outline model while
/// giving compact cards a single, useful first line.
extension ChapterDisplay on OutlineChapter {
  String? get decision => outlineDecision(decisions);

  String get when {
    final start = _shortDate(startedAt);
    if (start.isEmpty) return '';
    if (ongoing) return 'since $start';
    final end = _shortDate(endedAt ?? '');
    return end.isEmpty || end == start ? start : '$start – $end';
  }
}

extension SectionDisplay on OutlineSection {
  String? get decision => outlineDecision(decisions);
}

String _shortDate(String value) {
  final date = DateTime.tryParse(value);
  if (date == null) return '';
  const months = [
    'Jan',
    'Feb',
    'Mar',
    'Apr',
    'May',
    'Jun',
    'Jul',
    'Aug',
    'Sep',
    'Oct',
    'Nov',
    'Dec',
  ];
  return '${months[date.month - 1]} ${date.day}';
}
