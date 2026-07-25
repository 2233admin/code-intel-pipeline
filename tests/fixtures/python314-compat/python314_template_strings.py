name = "world"
template = t"hello {name}"

print(f"type={type(template).__name__}")
print(f"strings={template.strings}")
print(f"value={template.interpolations[0].value}")
