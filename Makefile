.PHONY: docs docs-open

docs:
	RUSTDOCFLAGS="--cfg docsrs" cargo +nightly doc --no-deps --all-features

docs-test:
	RUSTDOCFLAGS="--cfg docsrs" cargo +nightly test --doc --all-features

docs-open:
	RUSTDOCFLAGS="--cfg docsrs" cargo +nightly doc --no-deps --all-features --open

readme:
	cargo readme -i src/lib.rs -r round-based/ -t ../docs/readme.tpl --no-indent-headings \
		| perl -ne 's/(?<!!)\[([^\[]+?)\]\((?!https:)[^\(]+?\)/\1/g; print;' \
		| perl -ne 's/\[([^\[]+?)\](?!\()/\1/g; print;' \
		> README.md
