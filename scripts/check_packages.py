import json
from pathlib import Path
import subprocess
import sys


ALLOWED = {
    "vpdf-model": set(),
    "vpdf-protocol": set(),
    "vpdf-search": {"vpdf-model"},
    "vpdf-reader": {"vpdf-model", "vpdf-search"},
    "vpdf-pdf": {"vpdf-model", "vpdf-search"},
    "vpdf-terminal": {"vpdf-model", "vpdf-protocol"},
    "vpdf-ui": {"vpdf-model", "vpdf-reader", "vpdf-search", "vpdf-terminal"},
    "vpdf-gui": {"vpdf", "vpdf-model"},
    "vpdf": {
        "vpdf-model", "vpdf-reader", "vpdf-search", "vpdf-pdf",
        "vpdf-terminal", "vpdf-ui",
    },
}
NATIVE = {"vpdf", "vpdf-pdf", "vpdf-gui"}


def native(name):
    return name == "mupdf" or name.startswith("mupdf-")


def gui(name):
    return name in {"eframe", "egui", "epaint", "emath", "ecolor", "winit"} or name.startswith(("egui-", "wgpu"))


def reachable(graph, root):
    seen = set()
    pending = [root]
    while pending:
        node = pending.pop()
        if node not in seen:
            seen.add(node)
            pending.extend(graph[node])
    return seen


def audit(metadata):
    packages = {p["id"]: p for p in metadata["packages"]}
    members = metadata["workspace_members"]
    if any(member not in packages for member in members):
        return ["Workspace package metadata missing"]
    owners = {packages[member]["name"]: member for member in members}
    if set(owners) != set(ALLOWED) or len(owners) != len(members):
        return ["Workspace members differ from package policy"]
    if metadata["resolve"] is None:
        return ["Resolved dependency graph missing"]
    graph = {
        node["id"]: {dep["pkg"] for dep in node["deps"]}
        for node in metadata["resolve"]["nodes"]
    }
    referenced = set(members) | set(graph)
    referenced.update(dep for deps in graph.values() for dep in deps)
    if referenced - packages.keys() or referenced - graph.keys():
        return ["Resolved dependency metadata incomplete"]

    errors = []
    for name, member in sorted(owners.items()):
        for dep in packages[member]["dependencies"]:
            target = dep["name"]
            context = dep["kind"] or "normal"
            if dep["target"]:
                context += f", {dep['target']}"
            edge = f"{name} -> {target} ({context})"
            if target in ALLOWED:
                if target not in ALLOWED[name]:
                    errors.append(f"Forbidden dependency: {edge}")
                directory = Path(packages[owners[target]]["manifest_path"]).parent
                path = dep.get("path")
                if path is None or Path(path).resolve() != directory.resolve():
                    errors.append(f"Non-workspace dependency: {edge}")
            if name != "vpdf-gui" and gui(target):
                errors.append(f"GUI dependency: {edge}")
            if name not in NATIVE and native(target):
                errors.append(f"Native dependency: {edge}")

        permitted = reachable(ALLOWED, name)
        for target in sorted(reachable(graph, member)):
            dependency = packages[target]["name"]
            edge = f"{name} -> {dependency}"
            if dependency in ALLOWED:
                if dependency not in permitted:
                    errors.append(f"Forbidden transitive dependency: {edge}")
                if target != owners[dependency]:
                    errors.append(f"Non-workspace transitive dependency: {edge}")
            if name != "vpdf-gui" and gui(dependency):
                errors.append(f"GUI transitive dependency: {edge}")
            if name not in NATIVE and native(dependency):
                errors.append(f"Native transitive dependency: {edge}")
    return errors


def main():
    root = Path(__file__).resolve().parent.parent
    result = subprocess.run(
        ["cargo", "metadata", "--locked", "--all-features",
         "--format-version", "1"],
        cwd=root, stdout=subprocess.PIPE, text=True,
    )
    if result.returncode:
        return result.returncode
    errors = audit(json.loads(result.stdout))
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print("Package boundaries verified")
    return 0


if __name__ == "__main__":
    sys.exit(main())
