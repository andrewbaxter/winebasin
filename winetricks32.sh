# d3dx library shims are provided by wine, the winetricks versions are the full windows versions.
# The wine-provided versions are expected to be sufficient in the long term.
#
# dxvk/vkd3d are implementations for the shim libraries (?) - shims are separate.
#
# Major versions of all libraries are needed, not minor versions (with some exceptions).
winetricks -q \
	allfonts \
	allcodecs \
	dotnet11sp1 \
	dotnet11 \
	dotnet20sp1 \
	dotnet20sp2 \
	dotnet20 \
	dotnet30 \
	dotnet30sp1 \
	dotnet35 \
	dotnet35sp1 \
	dotnet40 \
	dotnet452 \
	dotnet462 \
	dotnet471 \
	dotnet472 \
	dotnet48 \
	dotnet6 \
	dotnet7 \
	dotnetcore2 \
	dotnetcore3 \
	dxvk2030 \
	vkd3d \
	vcrun2003 \
	vcrun2005 \
	vcrun2008 \
	vcrun2010 \
	vcrun2012 \
	vcrun2013 \
	vcrun2015 \
	vcrun2017 \
	vcrun2019 \
	vcrun2022 \
	vcrun6sp6 \
	vcrun6 \
	;
