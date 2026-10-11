"""check-no-typescript on passing and failing fixture trees (ADR-0036)."""


def test_hand_written_tsx_fails(write_tree, run_check):
    root = write_tree({"apps/web/src/sections/home/Home.tsx": "export function Home(): null { return null; }\n"})
    result = run_check("check-no-typescript", "--root", str(root))
    assert result.returncode == 1
    assert "apps/web/src/sections/home/Home.tsx: TypeScript file outside the generated folders" in result.stdout


def test_generated_declarations_pass(write_tree, run_check):
    root = write_tree(
        {
            "packages/proto/src/generated/rastai/commands/v1/commands_pb.d.ts": "export declare class Command {}\n",
            "packages/client/src/generated/commands.d.ts": "export declare function modelLoad(): void;\n",
            "plugins/psychology/ui/src/generated/types.d.ts": "export declare type Lens = string;\n",
            "packages/ui/src/Button.jsx": "export function Button() { return null; }\n",
            "node_modules/react/index.d.ts": "export {};\n",
            "spike/web/canvas.ts": "export {};\n",
        }
    )
    result = run_check("check-no-typescript", "--root", str(root))
    assert result.returncode == 0, result.stdout


def test_every_typescript_ending_outside_generated_folders_fails(write_tree, run_check):
    root = write_tree(
        {
            "packages/ui/src/types.d.ts": "export {};\n",
            "packages/client/src/connection.ts": "export {};\n",
            "tools/scripts/build.mts": "export {};\n",
            "packages/plugin/src/generated-by-hand.cts": "export {};\n",
        }
    )
    result = run_check("check-no-typescript", "--root", str(root))
    assert result.returncode == 1
    assert "4 problem(s)" in result.stdout


def test_the_real_repository_passes(run_check):
    result = run_check("check-no-typescript")
    assert result.returncode == 0, result.stdout
