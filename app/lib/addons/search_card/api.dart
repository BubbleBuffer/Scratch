// The search card add-on's entry point. The shell imports only this file.
// It shows a `query` block as the live results of its query. It needs no
// other add-on: with Tasks on, task rows get a checkbox that writes the Tasks
// status; with Tasks off they have none.

import '../../registry/addon_registry.dart';
import 'query_block.dart';
import 'query_inline.dart';

class SearchCardAddon extends Addon {
  const SearchCardAddon();
  @override
  String get id => 'search_card';
  @override
  List<String> get dependsOn => const [];
  @override
  void register(AddonContext ctx) {
    ctx.blockKinds.register(searchQueryBlock);
    ctx.inlineMarks.register(const QueryInlineMark());
    ctx.slashItems.register(
      SlashItem(
        id: searchQueryKind,
        label: 'Search query',
        description: 'Live results of a search, kept up to date.',
        group: 'Advanced',
        blockKind: searchQueryKind,
        run: (_) {},
      ),
    );
  }
}
