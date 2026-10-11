"""check-colors on passing and failing fixture trees (owner decision N1, contracts 5.6)."""

# The spike's hard-coded canvas background, the color the acceptance test names (plan M1-05).
SPIKE_CANVAS_BACKGROUND = "#101318"


def test_spike_color_in_a_canvas_file_fails(write_tree, run_check):
    root = write_tree({"packages/canvas/src/graph-renderer.js": f'const BACKGROUND = "{SPIKE_CANVAS_BACKGROUND}";\n'})
    result = run_check("check-colors", "--root", str(root))
    assert result.returncode == 1
    assert f"packages/canvas/src/graph-renderer.js:1: color literal `{SPIKE_CANVAS_BACKGROUND}`" in result.stdout


def test_the_same_color_in_the_token_folder_passes(write_tree, run_check):
    theme_source = f'export const DARK = {{ canvas_background: "{SPIKE_CANVAS_BACKGROUND}" }};\n'
    root = write_tree({"packages/ui/src/tokens/built-in-themes.js": theme_source})
    result = run_check("check-colors", "--root", str(root))
    assert result.returncode == 0, result.stdout


def test_passing_tree(write_tree, run_check):
    root = write_tree(
        {
            "packages/canvas/src/colors.js": "// The spike's #101318 is gone; colors arrive through setColors.\n"
            "export function packColors(colors) { return colors.background; }\n",
            "packages/viz/src/chart/line-chart.js": 'const LABEL = "Status: green";\nconst ROUTE = "#/settings";\n',
            "apps/web/src/sections/home/home.css": "/* not red */\n.home { color: var(--rr-color-text); }\n",
            "plugins/psychology/ui/src/LivePanel.jsx": 'export const PANEL = <div className="live" />;\n',
            "packages/proto/src/generated/colors_pb.js": 'export const EXAMPLE = "#ffffff";\n',
            "apps/web/src/app/tokens.test.js": "// tests outside tokens/ still use tokens\n",
            # Names that hold a color word, a url() path or fragment, and JSX text with an apostrophe.
            "apps/web/src/sections/runs/runs.css": ".tag:not(.green) {\n  background: url(icons/red-arrow.svg);\n"
            "  mask: url(#fade);\n  border-color: var(--rr-color-red);\n}\n",
            "apps/web/src/sections/runs/RunNote.jsx": "export const NOTE = <p>Don't reopen issue #123</p>;\n",
        }
    )
    result = run_check("check-colors", "--root", str(root))
    assert result.returncode == 0, result.stdout


def test_every_kind_of_color_literal_fails(write_tree, run_check):
    root = write_tree(
        {
            "packages/viz/src/heatmap.js": 'const LOW = "rgba(0, 0, 255, 0.5)";\n',
            "apps/web/src/sections/home/home.css": ".home {\n  border: 1px solid red;\n}\n",
            "plugins/evaluation/ui/src/Score.jsx": 'export const STYLE = { color: "white" };\n',
            "packages/ui/src/components/Button.jsx": 'export const HOVER = "hsl(210, 50%, 40%)";\n',
            "apps/web/src/app/shell.js": 'export const FALLBACK = "#fff";\n',
            "apps/web/src/app/banner.js": "export const BANNER_STYLE = `\n  color: red;\n`;\n",
            "apps/web/src/app/theme.css": ".shell {\n  color: var(--rr-color-text, black);\n}\n",
        }
    )
    result = run_check("check-colors", "--root", str(root))
    assert result.returncode == 1
    assert "packages/viz/src/heatmap.js:1: color literal `rgba(`" in result.stdout
    assert "apps/web/src/sections/home/home.css:2: color literal `red`" in result.stdout
    assert 'plugins/evaluation/ui/src/Score.jsx:1: color literal `"white"`' in result.stdout
    assert "packages/ui/src/components/Button.jsx:1: color literal `hsl(`" in result.stdout
    assert "apps/web/src/app/shell.js:1: color literal `#fff`" in result.stdout
    assert "apps/web/src/app/banner.js:2: color literal `red`" in result.stdout
    assert "apps/web/src/app/theme.css:2: color literal `black`" in result.stdout
    assert "7 problem(s)" in result.stdout


def test_the_real_repository_passes(run_check):
    result = run_check("check-colors")
    assert result.returncode == 0, result.stdout


def test_color_in_the_proto_package_entry_fails(write_tree, run_check):
    # @rastai/proto's index.js is hand-written, so only its src/generated/ is exempt.
    root = write_tree(
        {
            "packages/proto/src/generated/colors_pb.js": 'export const EXAMPLE = "#ffffff";\n',
            "packages/proto/src/index.js": f'export const BACKGROUND = "{SPIKE_CANVAS_BACKGROUND}";\n',
        }
    )
    result = run_check("check-colors", "--root", str(root))
    assert result.returncode == 1
    assert f"packages/proto/src/index.js:1: color literal `{SPIKE_CANVAS_BACKGROUND}`" in result.stdout
    assert "1 problem(s)" in result.stdout
