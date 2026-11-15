package Locale::Maketext::Simple;
use strict;
use vars qw($VERSION @EXPORT @ISA);
$VERSION = "0.21";
require Exporter;
@ISA = qw(Exporter);
@EXPORT = qw(loc loc_lang);
sub import {
    my $pkg = shift;
    my $callpkg = caller;
    no strict "refs";
    *{"${callpkg}::loc"} = \&loc;
    *{"${callpkg}::loc_lang"} = \&loc_lang;
}
sub loc { return sprintf(shift, @_); }
sub loc_lang { return "en"; }
1;
