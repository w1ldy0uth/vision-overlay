"""Create a portable ZIP with canonical POSIX entry names on every host OS."""
import argparse
from pathlib import Path
import zipfile


def package(directory, output):
    directory = directory.resolve(strict=True)
    if not directory.is_dir() or output.exists():
        raise ValueError("Expected a package directory and a new output file")
    files = sorted(p for p in directory.rglob("*") if p.is_file())
    with zipfile.ZipFile(output, "x", compression=zipfile.ZIP_DEFLATED,
                         compresslevel=6, strict_timestamps=False) as archive:
        for path in files:
            if path.is_symlink() or not path.resolve().is_relative_to(directory):
                raise ValueError("Package links must not escape the directory")
            name = path.relative_to(directory.parent).as_posix()
            archive.write(path, arcname=name)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    package(args.directory, args.output)
