import SwiftUI

/// The mirrored screen: coloured lines, following the bottom unless the
/// user has scrolled up to read.
struct ScreenView: View {
    let lines: [ScreenLine]

    @State private var follow = true
    @State private var seenViewport: CGFloat = -1
    @State private var seenLastLine: Int?

    private static let bottom = "bottom"
    private static let space = "screen"

    var body: some View {
        GeometryReader { outer in
            ScrollViewReader { proxy in
                ScrollView {
                    // A plain VStack, not a lazy one: the bottom marker must
                    // always exist to report where it is, and scrollTo must
                    // land exactly. Fine for a thousand lines.
                    VStack(alignment: .leading, spacing: 1) {
                        ForEach(lines) { line in
                            Text(line.text.isEmpty ? " " : line.text)
                                .foregroundStyle(line.tone.color)
                                .frame(maxWidth: .infinity, alignment: .leading)
                        }
                        Color.clear
                            .frame(height: 1)
                            .id(Self.bottom)
                            .background(GeometryReader { g in
                                Color.clear.preference(key: BottomEdge.self, value: BottomEdge.Edge(
                                    distance: g.frame(in: .named(Self.space)).maxY - outer.size.height,
                                    viewport: outer.size.height,
                                    lastLine: lines.last?.id
                                ))
                            })
                    }
                    .font(Theme.font)
                    .textSelection(.enabled)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 6)
                }
                .coordinateSpace(name: Self.space)
                // New lines, the keyboard, a rotation: stay at the bottom.
                .onChange(of: lines.last?.id) { _ in
                    if follow { proxy.scrollTo(Self.bottom, anchor: .bottom) }
                }
                .onChange(of: outer.size.height) { _ in
                    if follow { proxy.scrollTo(Self.bottom, anchor: .bottom) }
                }
                .onPreferenceChange(BottomEdge.self) { edge in
                    guard let edge else { return }
                    // Same lines, same viewport, yet the bottom moved: the
                    // user scrolled. Follow again once they are back down.
                    if edge.viewport == seenViewport, edge.lastLine == seenLastLine {
                        follow = edge.distance < 24
                    }
                    seenViewport = edge.viewport
                    seenLastLine = edge.lastLine
                }
                .overlay(alignment: .bottomTrailing) {
                    if !follow {
                        Button {
                            follow = true
                            proxy.scrollTo(Self.bottom, anchor: .bottom)
                        } label: {
                            Image(systemName: "arrow.down.circle.fill")
                                .font(.title)
                                .foregroundStyle(Tone.dim.color)
                        }
                        .padding(12)
                        .accessibilityLabel("Scroll to bottom")
                    }
                }
                .onAppear { proxy.scrollTo(Self.bottom, anchor: .bottom) }
            }
        }
    }
}

private struct BottomEdge: PreferenceKey {
    struct Edge: Equatable {
        var distance: CGFloat
        var viewport: CGFloat
        var lastLine: Int?
    }

    static let defaultValue: Edge? = nil

    static func reduce(value: inout Edge?, nextValue: () -> Edge?) {
        value = nextValue() ?? value
    }
}
