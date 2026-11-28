use crate::manifest::Manifest;

pub struct Entry {
    pub name: String,
    pub location: String,
}

pub fn collect(manifest: &Manifest) -> Vec<Entry> {
    let mut entries = Vec::new();

	for (name, file_name) in &manifest.binaries {
		entries.push(Entry {
			name: name.clone(),
			location: format!("bin/{file_name}"),
		});
	}

	for (package_name, package) in &manifest.packages {
		for (name, relative) in &package.entries {
			entries.push(Entry {
				name: name.clone(),
				location: format!("packages/{package_name}/{relative}"),
			});
		}
	}

    entries.sort_by(|a, b| a.name.cmp(&b.name));

    entries
}

pub fn print(entries: &[Entry], query: Option<&str>) {
    let name_width = entries
        .iter()
        .map(|entry| entry.name.chars().count())
        .max()
        .unwrap_or(4)
        .max(20);

    println!("\x1b[34m{:<name_width$} LOCATION\x1b[0m", "NAME");

    for entry in entries {
        let name = match query {
            Some(query) => highlight_matches(&entry.name, query),
            None => entry.name.clone(),
        };

        let padding = name_width.saturating_sub(entry.name.chars().count());

        println!("{}{} {}", name, " ".repeat(padding), entry.location);
    }
}

fn highlight_matches(name: &str, query: &str) -> String {
    let lower_name = name.to_ascii_lowercase();
    let lower_query = query.to_ascii_lowercase();

    if lower_query.is_empty() {
        return name.to_string();
    }

    let mut result = String::new();
    let mut remaining = 0;

    while let Some(pos) = lower_name[remaining..].find(&lower_query) {
        let start = remaining + pos;
        let end = start + lower_query.len();

        result.push_str(&name[remaining..start]);
        result.push_str("\x1b[1m");
        result.push_str(&name[start..end]);
        result.push_str("\x1b[0m");

        remaining = end;
    }

    result.push_str(&name[remaining..]);

    result
}
