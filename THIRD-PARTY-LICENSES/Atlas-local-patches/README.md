# Local dependency patches

`egui` and `epaint` are based on version 0.37.0 by Emil Ernerfeldt and the egui contributors, licensed under MIT OR Apache-2.0. The bidirectional text implementation is adapted from abdu-benayad’s upstream pull request [emilk/egui#8577](https://github.com/emilk/egui/pull/8577), revision `c93e8863e958507056415e2a6dace56e23ba5f3a`, under the same license. Original copyright and license notices are retained.

NIS adapted the patch to 0.37.0 and corrected cluster character bookkeeping, mark ownership, mixed-direction end cursors, discontiguous selection backgrounds, visual arrow navigation, and inline storage for short visual-order lists. Fontless glyph runs retain their bidi level. The egui patch covers selection painting and selection collapse in visual order.

`gpu-allocator` 0.28.0 is by Traverse Research, licensed under MIT OR Apache-2.0. Its local manifest selects the Windows 0.62.2 bindings used by wgpu 30; the original inclusive upper bound selected incompatible 0.61 bindings. The non-linear allocation enum case is compiled only when a backend that uses it is enabled. Allocation behavior is unchanged.

These crates are kept locally so builds do not depend on a moving fork. Remove the patches after equivalent fixes ship upstream and the regression tests pass against that version.
