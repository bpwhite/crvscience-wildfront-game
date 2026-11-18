# Preprints for WildFront BattleGrounds (`crvscience-wildfront-game`)

**Author:** Bryan P. White (ORCID: [0000-0003-4191-3511](https://orcid.org/0000-0003-4191-3511))  
**Hub:** https://crvscience.com/apps

This directory contains the self-contained LaTeX source (`.tex`) and single-column PDF (`.pdf`) editions for both:

1. **Companion Application Preprint (`APP-PREPRINT-12`)**
   - **Title:** WildFront BattleGrounds: Turn-Based Hex Strategy Game with Voxel Graphics and Procedural Map Generation
   - **Files:** `wildfront-hexagonal-ecological-tactics.tex`, `wildfront-hexagonal-ecological-tactics.pdf`
   - **Canonical Zenodo DOI:** https://doi.org/10.5281/zenodo.24222396
   - **Figshare Mirror:** https://figshare.com/articles/preprint/WildFront_BattleGrounds_Turn-Based_Hex_Strategy_Game_with_Voxel_Graphics_and_Procedural_Map_Generation/35013775
   - **Web Preprint:** https://crvscience.com/apps/preprints/wildfront-hexagonal-ecological-tactics

2. **CRV Science Public Apps Network Overview (`APP-PREPRINT-01`)**
   - **Title:** The CRV Science App Network: Eighteen Open-Source Rust Programs for Science, Data, and Simulation
   - **Files:** `crv-science-public-apps-network.tex`, `crv-science-public-apps-network.pdf`
   - **Canonical Zenodo DOI:** https://doi.org/10.5281/zenodo.24222398
   - **Figshare Mirror:** https://figshare.com/articles/preprint/The_CRV_Science_App_Network_Seventeen_Open-Source_Rust_Programs_for_Science_Data_and_Simulation/35013736
   - **Web Preprint:** https://crvscience.com/apps/preprints/crv-science-public-apps-network

## Compiling Standalone LaTeX

Both `.tex` files inline their bibliography so each compiles standalone with standard `pdflatex`:

```bash
pdflatex wildfront-hexagonal-ecological-tactics.tex
pdflatex crv-science-public-apps-network.tex
```
