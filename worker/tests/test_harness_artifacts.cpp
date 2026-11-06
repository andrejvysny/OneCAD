#include <filesystem>
#include <fstream>
#include <string>
#include <unistd.h>
#include "../tools/harness/ArtifactPlaceholders.h"

int main() {
    char pattern[] = "/tmp/onecad-harness-test-XXXXXX";
    const char* tmp = mkdtemp(pattern);
    if (!tmp) return 1;
    const std::filesystem::path root(tmp);
    nlohmann::json missing = "$sha256:sample";
    nlohmann::json outside = "$sha256:../sample";
    nlohmann::json malformed = "$artifact:";
    nlohmann::json bad_prefix = "$sha256";
    const bool negative = !onecad::harness::expand_artifacts(bad_prefix, root) && !onecad::harness::expand_artifacts(missing, root) &&
        !onecad::harness::expand_artifacts(outside, root) &&
        !onecad::harness::expand_artifacts(malformed, root);
    { std::ofstream out(root / "sample"); out << "abc"; }
    nlohmann::json values = {{"path", "$artifact:sample"}, {"digest", "$sha256:sample"}};
    const bool positive = onecad::harness::expand_artifacts(values, root) &&
        values["path"] == (root / "sample").string() &&
        values["digest"] == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    std::filesystem::create_symlink(root / "sample", root / "link");
    nlohmann::json link = "$sha256:link";
    const bool no_link = !onecad::harness::expand_artifacts(link, root);
    const auto large_path = root / "large";
    { std::ofstream out(large_path); out.put('x'); }
    std::filesystem::resize_file(large_path, onecad::io::kMaxImportSourceBytes + 1);
    nlohmann::json large = "$sha256:large";
    const bool no_large = !onecad::harness::expand_artifacts(large, root);
    std::filesystem::remove_all(root);
    return negative && positive && no_link && no_large ? 0 : 1;
}
