"""Exercise actual hash-installed YAML behavior; missing PyYAML is an error."""


def main():
    import yaml
    print(f"Actual PyYAML version: {yaml.__version__}", flush=True)
    valid = "name: dependency-smoke\ndescription: safe frontmatter\n"
    expected = {"name": "dependency-smoke", "description": "safe frontmatter"}
    if yaml.safe_load(valid) != expected:
        raise AssertionError("Valid frontmatter changed meaning")
    for invalid in ["name: [unterminated", "!!python/object/apply:builtins.eval ['1 + 1']"]:
        try:
            yaml.safe_load(invalid)
        except yaml.YAMLError:
            continue
        raise AssertionError("Malformed or executable frontmatter was accepted")
    print("PASS valid frontmatter; malformed and executable YAML rejected", flush=True)


if __name__ == "__main__":
    main()
